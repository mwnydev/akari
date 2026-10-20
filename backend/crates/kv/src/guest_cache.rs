//! 游客响应缓存（plan 第 8 节「游客页面」，在 Cloudflare 60 秒边缘缓存之后的第二层）。
//!
//! 只存与访问者无关的游客响应（调用方保证：带登录会话的请求一律不经过这里）。
//!
//! - **版本标签**：每个条目记下写入时它依赖的各版本号（如 `forums`、`gboard:3`、`gtopic:42`），写操作给对应版本号
//!   加一即精确失效。读者拿当前版本与条目的标签比较，不一致就当作不存在，**绝不返回写入之前的内容**。
//! - **单飞**：未命中时只有拿到锁的那个请求回源，其余请求短暂等待它写回（[`GuestCache::lookup`] 返回 [`Lookup::Wait`]），
//!   热门页面失效的瞬间不会有成百个请求同时打到数据库。
//! - **过期后先用旧值**（stale-while-revalidate）：条目的新鲜期过了但版本没变时，一个请求回源重建，
//!   其余请求直接拿旧值。只按 TTL 刷新的内容（名片、浏览数、库存）因此最多晚一个 [`GuestTtl::stale`]。
//!
//! 版本号、条目与锁都在 `{ns}g…` 下；时间取 Valkey 服务器时间（多实例之间不受本机时钟偏差影响）。

use std::sync::Arc;
use std::time::Duration;

use redis::Script;

use crate::{KvError, Pool};

/// 回源锁的最长持有时间：远长于一次回源，持有者崩溃时到期自动放开。
const LOCK_MS: u64 = 3_000;
/// 版本号闲置多久过期（远长于条目的硬过期时间，过期后从 0 重新计数也匹配不上任何残留条目）。
const VERSION_TTL_SECS: u64 = 86_400;

const LOOKUP: &str = r"
local tag = {}
for i = 3, #KEYS do tag[#tag + 1] = redis.call('GET', KEYS[i]) or '0' end
tag = table.concat(tag, '.')
local e = redis.call('HMGET', KEYS[1], 'tag', 'fresh', 'body')
local t = redis.call('TIME')
local now = tonumber(t[1]) * 1000 + math.floor(tonumber(t[2]) / 1000)
local current = e[3] and e[1] == tag
if current and tonumber(e[2]) > now then return {'hit', e[3]} end
if redis.call('SET', KEYS[2], ARGV[1], 'NX', 'PX', ARGV[2]) then return {'build', tag} end
if current then return {'stale', e[3]} end
return {'wait', ''}
";

const STORE: &str = r"
local t = redis.call('TIME')
local now = tonumber(t[1]) * 1000 + math.floor(tonumber(t[2]) / 1000)
redis.call('HSET', KEYS[1], 'tag', ARGV[1], 'fresh', now + tonumber(ARGV[2]), 'body', ARGV[4])
redis.call('EXPIRE', KEYS[1], ARGV[3])
if redis.call('GET', KEYS[2]) == ARGV[5] then redis.call('DEL', KEYS[2]) end
return 1
";

/// 放开回源锁；`ARGV[2]` 为 1 时同时删除条目（回源发现内容已不存在，例如主题被隐藏）。
const RELEASE: &str = r"
if ARGV[2] == '1' then redis.call('DEL', KEYS[1]) end
if redis.call('GET', KEYS[2]) == ARGV[1] then redis.call('DEL', KEYS[2]) end
return 1
";

/// 条目的新鲜期与过期后仍可当旧值用的时长；硬过期 = 两者之和。
#[derive(Debug, Clone, Copy)]
pub struct GuestTtl {
    pub fresh: Duration,
    pub stale: Duration,
}

/// 查一次的结果。
#[derive(Debug)]
pub enum Lookup {
    /// 当前版本、仍新鲜。
    Hit(String),
    /// 当前版本、已过新鲜期，另有请求正在重建。
    Stale(String),
    /// 本请求拿到了回源锁：回源后用 [`GuestCache::store`] 写回，失败时 [`GuestCache::release`]。
    Build(Ticket),
    /// 另一个请求正在回源，且没有可用的当前版本。
    Wait,
}

/// 回源锁凭证与查到时的版本标签（回源前读取的版本：回源期间有写入时，写回的条目标签已过时，不会被当作命中）。
#[derive(Debug)]
pub struct Ticket {
    tag: String,
    token: String,
}

#[derive(Clone)]
pub struct GuestCache {
    pool: Pool,
    ns: Arc<str>,
}

impl std::fmt::Debug for GuestCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuestCache")
            .field("ns", &self.ns)
            .finish_non_exhaustive()
    }
}

impl GuestCache {
    /// `ns` 同 [`crate::ForumCache::new`]。
    pub fn new(pool: Pool, ns: impl Into<Arc<str>>) -> Self {
        Self {
            pool,
            ns: ns.into(),
        }
    }

    fn entry_key(&self, entry: &str) -> String {
        format!("{}g:{entry}", self.ns)
    }

    fn lock_key(&self, entry: &str) -> String {
        format!("{}g:{entry}:lock", self.ns)
    }

    fn version_key(&self, version: &str) -> String {
        format!("{}gv:{version}", self.ns)
    }

    /// 查条目 `entry`；`versions` 是它依赖的版本号名（顺序固定，见 [`crate::guest_cache`] 各名称函数）。
    pub async fn lookup(&self, entry: &str, versions: &[String]) -> Result<Lookup, KvError> {
        let mut conn = self.pool.get().await?;
        let token = format!("{:032x}", rand::random::<u128>());
        let script = Script::new(LOOKUP);
        let mut script = script.prepare_invoke();
        script.key(self.entry_key(entry)).key(self.lock_key(entry));
        for v in versions {
            script.key(self.version_key(v));
        }
        let (kind, value): (String, String) = script
            .arg(&token)
            .arg(LOCK_MS)
            .invoke_async(&mut conn)
            .await?;
        Ok(match kind.as_str() {
            "hit" => Lookup::Hit(value),
            "stale" => Lookup::Stale(value),
            "build" => Lookup::Build(Ticket { tag: value, token }),
            _ => Lookup::Wait,
        })
    }

    /// 写回回源结果并放开锁。
    pub async fn store(
        &self,
        entry: &str,
        ticket: Ticket,
        body: &str,
        ttl: GuestTtl,
    ) -> Result<(), KvError> {
        let mut conn = self.pool.get().await?;
        let fresh_ms = u64::try_from(ttl.fresh.as_millis()).unwrap_or(u64::MAX);
        let hard = (ttl.fresh + ttl.stale).as_secs().max(1);
        Script::new(STORE)
            .key(self.entry_key(entry))
            .key(self.lock_key(entry))
            .arg(&ticket.tag)
            .arg(fresh_ms)
            .arg(hard)
            .arg(body)
            .arg(&ticket.token)
            .invoke_async::<i64>(&mut conn)
            .await?;
        Ok(())
    }

    /// 回源失败时放开锁；`gone` 为真（内容已不存在或不再对游客可见）时同时删除条目，旧值不再当作过期内容返回。
    pub async fn release(&self, entry: &str, ticket: Ticket, gone: bool) -> Result<(), KvError> {
        let mut conn = self.pool.get().await?;
        Script::new(RELEASE)
            .key(self.entry_key(entry))
            .key(self.lock_key(entry))
            .arg(&ticket.token)
            .arg(if gone { "1" } else { "0" })
            .invoke_async::<i64>(&mut conn)
            .await?;
        Ok(())
    }

    /// 失效：这些版本号各加一。
    pub async fn bump(&self, versions: &[String]) -> Result<(), KvError> {
        if versions.is_empty() {
            return Ok(());
        }
        let mut conn = self.pool.get().await?;
        let mut pipe = redis::pipe();
        pipe.atomic();
        for v in versions {
            let key = self.version_key(v);
            pipe.incr(&key, 1)
                .ignore()
                .expire(&key, VERSION_TTL_SECS as i64)
                .ignore();
        }
        pipe.query_async::<()>(&mut conn).await?;
        Ok(())
    }
}

/// 版本号：板块列表与板块可见性（后台改板块时加一，所有游客条目都依赖它）。
pub fn forums() -> String {
    "forums".to_owned()
}

/// 版本号：某板块的主题列表（发帖、回帖、置顶、隐藏、移动、改标题）。
pub fn board(forum_id: i32) -> String {
    format!("board:{forum_id}")
}

/// 版本号：跨板块最新主题（任何板块的列表变化）。
pub fn latest() -> String {
    "latest".to_owned()
}

/// 版本号：某主题的所有页（回帖、编辑、删除、恢复、点赞、评论、主题管理）。
pub fn topic(topic_id: i64) -> String {
    format!("topic:{topic_id}")
}

/// 主题在列表里的样子变化时要加一的版本号：该主题、首页与它所在（或刚离开）的板块。
/// forum-api 的写操作与 forum-worker、forum-indexer 的拍卖状态变化共用。
pub fn listing(topic_id: i64, forum_ids: &[i32]) -> Vec<String> {
    let mut versions = vec![topic(topic_id), latest()];
    versions.extend(forum_ids.iter().map(|&f| board(f)));
    versions
}
