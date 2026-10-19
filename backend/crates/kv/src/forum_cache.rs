//! 论坛缓存（plan 第 8 节「缓存（Valkey）」）。
//!
//! - **板块有序集合**：每板块一个 ZSET，成员 = 主题 ID，分数 = `last_post_at`（微秒），只放非置顶、非隐藏主题，
//!   保留最新 [`BOARD_CAP`] 个。另有一个「已建好」标记：标记不在时读者回源重建。重建是**合并**写入，
//!   与并发的发帖 / 回帖不会互相覆盖；读者按主键取回主题后再按标志与板块过滤，残留的隐藏 / 移走主题不会显示。
//! - **版本化缓存**（主题第 1 页、置顶列表）：失效 = 版本号加一，值存在 `键:版本` 下。
//!   读者拿着旧版本回源写回的值没人再读，避免「先读旧数据、后写回缓存」的竞态把旧页面留在缓存里。
//! - **跨板块最新主题**：一个全站 ZSET，成员与分数同板块有序集合，在同样的时机（发帖、回帖、置顶、隐藏、移动）
//!   与板块有序集合在同一个脚本里更新，也保留最新 [`BOARD_CAP`] 个、有自己的「已建好」标记。
//! - **浏览数**：先 `HINCRBY` 记在一个哈希里，worker 每 10 秒原子取走并批量写库。
//! - **用户名片**：每用户一个哈希（用户名、头像、等级、信誉），未命中回源后写回，带 TTL 兜底。
//!   资料变化时删除并留一个几秒的「刚失效」标记，回源写回遇到标记就放弃，避免先读旧数据、后写回的竞态。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use redis::Script;

use crate::{KvError, Pool};

/// 每板块有序集合最多保留的主题数（前 200 页 × 每页 50）。
pub const BOARD_CAP: usize = 10_000;
/// 版本号闲置多久过期；远长于缓存值的 TTL，过期后从 0 重新计数也不会读到旧值。
const VERSION_TTL_SECS: i64 = 86_400;

const BOARD_PAGE: &str = r"
if redis.call('EXISTS', KEYS[2]) == 0 then return {-1, {}} end
return {redis.call('ZCARD', KEYS[1]), redis.call('ZREVRANGE', KEYS[1], ARGV[1], ARGV[2], 'WITHSCORES')}
";

const BOARD_TOUCH: &str = r"
for i = 1, 2 do
  redis.call('ZADD', KEYS[i], ARGV[1], ARGV[2])
  redis.call('ZREMRANGEBYRANK', KEYS[i], 0, -(tonumber(ARGV[3]) + 1))
end
return 1
";

const LATEST_PAGE: &str = r"
if redis.call('EXISTS', KEYS[2]) == 0 then return {-1, {}} end
return {redis.call('ZCARD', KEYS[1]),
        redis.call('ZREVRANGEBYSCORE', KEYS[1], ARGV[1], '-inf', 'WITHSCORES', 'LIMIT', 0, ARGV[2])}
";

const CARD_FILL: &str = r"
if redis.call('EXISTS', KEYS[2]) == 1 then return 0 end
redis.call('DEL', KEYS[1])
redis.call('HSET', KEYS[1], unpack(ARGV, 2))
redis.call('EXPIRE', KEYS[1], ARGV[1])
return 1
";

/// 名片缓存兜底过期时间。
const CARD_TTL_SECS: u64 = 3600;
/// 名片失效后多久内不接受回源写回（远长于一次回源查询）。
const CARD_TOMBSTONE_SECS: u64 = 5;

const VERSIONED_GET: &str = r"
local v = redis.call('GET', KEYS[1]) or '0'
return {v, redis.call('GET', ARGV[1] .. v)}
";

const TAKE_VIEWS: &str = r"
local v = redis.call('HGETALL', KEYS[1])
redis.call('DEL', KEYS[1])
return v
";

/// 板块有序集合的一页。
#[derive(Debug, Clone, PartialEq)]
pub struct BoardPage {
    /// `(主题 ID, last_post_at 微秒)`，新的在前。
    pub entries: Vec<(i64, i64)>,
    /// 有序集合里的总数。
    pub total: u64,
}

#[derive(Clone)]
pub struct ForumCache {
    pool: Pool,
    ns: Arc<str>,
}

impl std::fmt::Debug for ForumCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForumCache")
            .field("ns", &self.ns)
            .finish_non_exhaustive()
    }
}

impl ForumCache {
    /// `ns` 是所有键的前缀（[`crate::namespace_from_env`]；测试用随机前缀隔离）。
    pub fn new(pool: Pool, ns: impl Into<Arc<str>>) -> Self {
        Self {
            pool,
            ns: ns.into(),
        }
    }

    pub fn ns(&self) -> &str {
        &self.ns
    }

    pub fn pool(&self) -> &Pool {
        &self.pool
    }

    fn board_key(&self, forum_id: i32) -> String {
        format!("{}board:{forum_id}:z", self.ns)
    }

    fn board_built_key(&self, forum_id: i32) -> String {
        format!("{}board:{forum_id}:built", self.ns)
    }

    fn latest_key(&self) -> String {
        format!("{}latest:z", self.ns)
    }

    fn latest_built_key(&self) -> String {
        format!("{}latest:built", self.ns)
    }

    fn card_key(&self, user_id: i64) -> String {
        format!("{}card:{user_id}", self.ns)
    }

    fn card_tombstone_key(&self, user_id: i64) -> String {
        format!("{}card:{user_id}:inv", self.ns)
    }

    fn views_key(&self) -> String {
        format!("{}views:pending", self.ns)
    }

    /// 读一页（`start`/`stop` 为名次，含两端）。有序集合还没建好时返回 `None`。
    pub async fn board_page(
        &self,
        forum_id: i32,
        start: usize,
        stop: usize,
    ) -> Result<Option<BoardPage>, KvError> {
        let mut conn = self.pool.get().await?;
        let (total, flat): (i64, Vec<String>) = Script::new(BOARD_PAGE)
            .key(self.board_key(forum_id))
            .key(self.board_built_key(forum_id))
            .arg(start)
            .arg(stop)
            .invoke_async(&mut conn)
            .await?;
        Ok(board_page_of(total, &flat))
    }

    /// 合并写入重建结果并打上「已建好」标记。
    pub async fn board_fill(&self, forum_id: i32, entries: &[(i64, i64)]) -> Result<(), KvError> {
        let mut conn = self.pool.get().await?;
        let key = self.board_key(forum_id);
        let mut pipe = redis::pipe();
        pipe.atomic();
        for chunk in entries.chunks(1000) {
            let items: Vec<(i64, i64)> = chunk.iter().map(|&(id, score)| (score, id)).collect();
            pipe.zadd_multiple(&key, &items).ignore();
        }
        pipe.zremrangebyrank(&key, 0, -(BOARD_CAP as isize + 1))
            .ignore()
            .set(self.board_built_key(forum_id), 1)
            .ignore();
        pipe.query_async::<()>(&mut conn).await?;
        Ok(())
    }

    /// 新主题或新回复：写入（或更新分数）板块与跨板块最新有序集合并裁剪到上限。标记不在时也写，重建时会合并。
    pub async fn board_touch(
        &self,
        forum_id: i32,
        topic_id: i64,
        score: i64,
    ) -> Result<(), KvError> {
        let mut conn = self.pool.get().await?;
        Script::new(BOARD_TOUCH)
            .key(self.board_key(forum_id))
            .key(self.latest_key())
            .arg(score)
            .arg(topic_id)
            .arg(BOARD_CAP)
            .invoke_async::<i64>(&mut conn)
            .await?;
        Ok(())
    }

    /// 置顶、隐藏、移出板块时从板块与跨板块最新有序集合移除（移到别的板块时随后的 `board_touch` 会加回最新）。
    pub async fn board_remove(&self, forum_id: i32, topic_id: i64) -> Result<(), KvError> {
        let mut conn = self.pool.get().await?;
        redis::pipe()
            .atomic()
            .zrem(self.board_key(forum_id), topic_id)
            .ignore()
            .zrem(self.latest_key(), topic_id)
            .ignore()
            .query_async::<()>(&mut conn)
            .await?;
        Ok(())
    }

    /// 跨板块最新主题：分数不超过 `max_score` 的最多 `count` 个，新的在前。还没建好时返回 `None`。
    /// 分数相同的成员按成员字符串排序，调用方按 `(分数, ID)` 游标过滤时要多取几个。
    pub async fn latest_page(
        &self,
        max_score: Option<i64>,
        count: usize,
    ) -> Result<Option<BoardPage>, KvError> {
        let mut conn = self.pool.get().await?;
        let max = max_score.map_or_else(|| "+inf".to_owned(), |s| s.to_string());
        let (total, flat): (i64, Vec<String>) = Script::new(LATEST_PAGE)
            .key(self.latest_key())
            .key(self.latest_built_key())
            .arg(max)
            .arg(count)
            .invoke_async(&mut conn)
            .await?;
        Ok(board_page_of(total, &flat))
    }

    /// 合并写入跨板块最新的重建结果并打上「已建好」标记。
    pub async fn latest_fill(&self, entries: &[(i64, i64)]) -> Result<(), KvError> {
        let mut conn = self.pool.get().await?;
        let key = self.latest_key();
        let mut pipe = redis::pipe();
        pipe.atomic();
        for chunk in entries.chunks(1000) {
            let items: Vec<(i64, i64)> = chunk.iter().map(|&(id, score)| (score, id)).collect();
            pipe.zadd_multiple(&key, &items).ignore();
        }
        pipe.zremrangebyrank(&key, 0, -(BOARD_CAP as isize + 1))
            .ignore()
            .set(self.latest_built_key(), 1)
            .ignore();
        pipe.query_async::<()>(&mut conn).await?;
        Ok(())
    }

    /// 批量读名片哈希，顺序与 `ids` 一致；未缓存的为 `None`。
    pub async fn cards_get(
        &self,
        ids: &[i64],
    ) -> Result<Vec<Option<HashMap<String, String>>>, KvError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut conn = self.pool.get().await?;
        let mut pipe = redis::pipe();
        for &id in ids {
            pipe.hgetall(self.card_key(id));
        }
        let maps: Vec<HashMap<String, String>> = pipe.query_async(&mut conn).await?;
        Ok(maps
            .into_iter()
            .map(|m| (!m.is_empty()).then_some(m))
            .collect())
    }

    /// 回源后写回名片（每个至少一个字段）；刚失效的用户跳过。
    pub async fn cards_put(&self, cards: &[(i64, Vec<(&str, String)>)]) -> Result<(), KvError> {
        if cards.is_empty() {
            return Ok(());
        }
        let mut conn = self.pool.get().await?;
        // 管道里用 EVAL（EVALSHA 在脚本未加载时会整批失败）；脚本很短
        let mut pipe = redis::pipe();
        for (id, fields) in cards {
            if fields.is_empty() {
                continue;
            }
            let cmd = pipe
                .cmd("EVAL")
                .arg(CARD_FILL)
                .arg(2)
                .arg(self.card_key(*id))
                .arg(self.card_tombstone_key(*id))
                .arg(CARD_TTL_SECS);
            for (k, v) in fields {
                cmd.arg(*k).arg(v);
            }
            cmd.ignore();
        }
        pipe.query_async::<()>(&mut conn).await?;
        Ok(())
    }

    /// 用户名、头像、等级或信誉变化后删除名片缓存。
    pub async fn cards_invalidate(&self, ids: &[i64]) -> Result<(), KvError> {
        if ids.is_empty() {
            return Ok(());
        }
        let mut conn = self.pool.get().await?;
        let mut pipe = redis::pipe();
        pipe.atomic();
        for &id in ids {
            pipe.del(self.card_key(id))
                .ignore()
                .set_ex(self.card_tombstone_key(id), 1, CARD_TOMBSTONE_SECS)
                .ignore();
        }
        pipe.query_async::<()>(&mut conn).await?;
        Ok(())
    }

    /// 读版本化缓存：返回当前版本与该版本下的值。
    pub async fn versioned_get(&self, name: &str) -> Result<(String, Option<String>), KvError> {
        let mut conn = self.pool.get().await?;
        let (version, value): (String, Option<String>) = Script::new(VERSIONED_GET)
            .key(format!("{}ver:{name}", self.ns))
            .arg(format!("{}c:{name}:", self.ns))
            .invoke_async(&mut conn)
            .await?;
        Ok((version, value))
    }

    /// 写入某版本下的值（版本取自之前的 [`Self::versioned_get`]）。
    pub async fn versioned_set(
        &self,
        name: &str,
        version: &str,
        value: &str,
        ttl: Duration,
    ) -> Result<(), KvError> {
        let mut conn = self.pool.get().await?;
        redis::cmd("SET")
            .arg(format!("{}c:{name}:{version}", self.ns))
            .arg(value)
            .arg("EX")
            .arg(ttl.as_secs().max(1))
            .query_async::<()>(&mut conn)
            .await?;
        Ok(())
    }

    /// 失效：版本号加一。
    pub async fn bump(&self, name: &str) -> Result<(), KvError> {
        let mut conn = self.pool.get().await?;
        let key = format!("{}ver:{name}", self.ns);
        redis::pipe()
            .atomic()
            .incr(&key, 1)
            .ignore()
            .expire(&key, VERSION_TTL_SECS)
            .ignore()
            .query_async::<()>(&mut conn)
            .await?;
        Ok(())
    }

    pub async fn add_view(&self, topic_id: i64) -> Result<(), KvError> {
        let mut conn = self.pool.get().await?;
        redis::cmd("HINCRBY")
            .arg(self.views_key())
            .arg(topic_id)
            .arg(1)
            .query_async::<i64>(&mut conn)
            .await?;
        Ok(())
    }

    /// 原子取走全部待写的浏览数。
    pub async fn take_views(&self) -> Result<Vec<(i64, i64)>, KvError> {
        let mut conn = self.pool.get().await?;
        let flat: Vec<String> = Script::new(TAKE_VIEWS)
            .key(self.views_key())
            .invoke_async(&mut conn)
            .await?;
        Ok(flat
            .as_chunks::<2>()
            .0
            .iter()
            .filter_map(|[id, n]| Some((id.parse().ok()?, n.parse().ok()?)))
            .collect())
    }

    /// 写库失败时把取走的浏览数加回去，下次再写。
    pub async fn restore_views(&self, entries: &[(i64, i64)]) -> Result<(), KvError> {
        if entries.is_empty() {
            return Ok(());
        }
        let mut conn = self.pool.get().await?;
        let key = self.views_key();
        let mut pipe = redis::pipe();
        for &(id, n) in entries {
            pipe.hincr(&key, id, n).ignore();
        }
        pipe.query_async::<()>(&mut conn).await?;
        Ok(())
    }
}

/// 解析 `{总数, [成员, 分数, …]}`；总数为 −1（还没建好）时返回 `None`。
fn board_page_of(total: i64, flat: &[String]) -> Option<BoardPage> {
    let total = u64::try_from(total).ok()?;
    let entries = flat
        .as_chunks::<2>()
        .0
        .iter()
        .filter_map(|[id, score]| {
            // 分数是微秒时间戳（< 2^53，f64 精确表示）
            #[allow(clippy::cast_possible_truncation)]
            let score = score.parse::<f64>().ok()? as i64;
            Some((id.parse::<i64>().ok()?, score))
        })
        .collect();
    Some(BoardPage { entries, total })
}

/// 缓存名：主题第 1 页。
pub fn topic_page1(topic_id: i64) -> String {
    format!("topic1:{topic_id}")
}

/// 缓存名：用户的主题数与帖子数（个人主页，只按 TTL 过期）。
pub fn user_counts(user_id: i64) -> String {
    format!("ucount:{user_id}")
}

/// 缓存名：板块置顶列表。
pub fn pinned(forum_id: i32) -> String {
    format!("pinned:{forum_id}")
}
