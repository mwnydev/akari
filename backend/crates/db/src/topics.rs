//! 主题。热路径查询放在 `queries/*.sql`，测试用同一份 SQL 做 EXPLAIN 断言。

use serde_json::Value;
use sqlx::PgExecutor;
use time::OffsetDateTime;

pub const FLAG_PINNED: i16 = 1;
pub const FLAG_FEATURED: i16 = 2;
pub const FLAG_LOCKED: i16 = 4;
pub const FLAG_HIDDEN: i16 = 8;
/// 不进普通列表的标志：置顶（单独展示）与隐藏。
pub const FLAGS_NOT_LISTED: i16 = FLAG_PINNED | FLAG_HIDDEN;

/// `topics.kind`：普通 / 交易 / 争议。
pub const KIND_NORMAL: i16 = 0;
pub const KIND_TRADE: i16 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Topic {
    pub id: i64,
    pub forum_id: i32,
    pub author_id: i64,
    pub title: String,
    pub kind: i16,
    pub flags: i16,
    pub last_floor: i32,
    pub last_post_id: Option<i64>,
    pub last_poster_id: Option<i64>,
    pub last_post_at: OffsetDateTime,
    pub created_at: OffsetDateTime,
    pub view_count: i64,
    /// 楼主之外最近发言的最多 4 个人，新的在前（回帖时维护）。
    pub recent_posters: Vec<i64>,
    /// 交易帖（kind = 交易）的交易帖 ID、价格（USDT 最小单位，十进制字符串）、卖家参考汇率与是否自动发货。
    pub listing_id: Option<i64>,
    pub price: Option<String>,
    pub cny_per_usdt: Option<i32>,
    pub instant: Option<bool>,
}

impl Topic {
    pub fn listed(&self) -> bool {
        self.flags & FLAGS_NOT_LISTED == 0
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Created {
    pub topic_id: i64,
    pub post_id: i64,
    pub last_post_at: OffsetDateTime,
}

/// 新主题的内容；HTML 已经过滤。
#[derive(Debug, Clone, Copy)]
pub struct NewTopic<'a> {
    pub forum_id: i32,
    pub author_id: i64,
    pub title: &'a str,
    pub html: &'a str,
    pub plain_text: &'a str,
    pub source: &'a Value,
    pub mentions: &'a [i64],
    pub kind: i16,
}

pub async fn create(db: impl PgExecutor<'_>, t: NewTopic<'_>) -> Result<Created, sqlx::Error> {
    let row = sqlx::query_file!(
        "queries/topic_create.sql",
        t.forum_id,
        t.author_id,
        t.title,
        t.html,
        t.plain_text,
        t.source,
        t.mentions,
        t.kind
    )
    .fetch_one(db)
    .await?;
    Ok(Created {
        topic_id: row.topic_id,
        post_id: row.post_id,
        last_post_at: row.last_post_at,
    })
}

pub async fn by_ids(db: impl PgExecutor<'_>, ids: &[i64]) -> Result<Vec<Topic>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_file_as!(Topic, "queries/topics_by_ids.sql", ids)
        .fetch_all(db)
        .await
}

pub async fn by_id(db: impl PgExecutor<'_>, id: i64) -> Result<Option<Topic>, sqlx::Error> {
    Ok(by_ids(db, &[id]).await?.pop())
}

/// 游标深分页。
pub async fn list_before(
    db: impl PgExecutor<'_>,
    forum_id: i32,
    before: (OffsetDateTime, i64),
    limit: i64,
) -> Result<Vec<Topic>, sqlx::Error> {
    sqlx::query_file_as!(
        Topic,
        "queries/topics_list_cursor.sql",
        forum_id,
        before.0,
        before.1,
        limit
    )
    .fetch_all(db)
    .await
}

/// 跨板块最新主题超出有序集合之后的深分页，只取 `forum_ids` 里的板块。
pub async fn latest_before(
    db: impl PgExecutor<'_>,
    forum_ids: &[i32],
    before: (OffsetDateTime, i64),
    limit: i64,
) -> Result<Vec<Topic>, sqlx::Error> {
    sqlx::query_file_as!(
        Topic,
        "queries/topics_latest_cursor.sql",
        forum_ids,
        before.0,
        before.1,
        limit
    )
    .fetch_all(db)
    .await
}

/// 重建跨板块最新主题有序集合用：全站最新 `limit` 个普通主题的 `(id, last_post_at)`。
pub async fn latest_listed_all(
    db: impl PgExecutor<'_>,
    limit: i64,
) -> Result<Vec<(i64, OffsetDateTime)>, sqlx::Error> {
    let rows = sqlx::query_file!("queries/topics_latest_rebuild.sql", limit)
        .fetch_all(db)
        .await?;
    Ok(rows.into_iter().map(|r| (r.id, r.last_post_at)).collect())
}

/// 重建板块有序集合用：最新 `limit` 个普通主题的 `(id, last_post_at)`。
pub async fn latest_listed(
    db: impl PgExecutor<'_>,
    forum_id: i32,
    limit: i64,
) -> Result<Vec<(i64, OffsetDateTime)>, sqlx::Error> {
    let rows = sqlx::query_file!("queries/topics_zset_rebuild.sql", forum_id, limit)
        .fetch_all(db)
        .await?;
    Ok(rows.into_iter().map(|r| (r.id, r.last_post_at)).collect())
}

pub async fn pinned_ids(db: impl PgExecutor<'_>, forum_id: i32) -> Result<Vec<i64>, sqlx::Error> {
    let rows = sqlx::query_file!("queries/topics_pinned.sql", forum_id)
        .fetch_all(db)
        .await?;
    Ok(rows.into_iter().map(|r| r.id).collect())
}

/// 设置 / 清除标志位，返回 `(旧标志, 新标志, 板块)`；主题不存在返回 `None`。
pub async fn update_flags(
    db: impl PgExecutor<'_>,
    id: i64,
    set: i16,
    clear: i16,
) -> Result<Option<(i16, i16, i32)>, sqlx::Error> {
    let row = sqlx::query_file!("queries/topic_update_flags.sql", id, set, clear)
        .fetch_optional(db)
        .await?;
    Ok(row.map(|r| (r.old, r.new, r.forum_id)))
}

/// 移动到另一个板块，返回原板块。
pub async fn move_to(
    db: impl PgExecutor<'_>,
    id: i64,
    forum_id: i32,
) -> Result<Option<i32>, sqlx::Error> {
    let row = sqlx::query_file!("queries/topic_move.sql", id, forum_id)
        .fetch_optional(db)
        .await?;
    Ok(row.map(|r| r.old))
}

pub async fn set_title(db: impl PgExecutor<'_>, id: i64, title: &str) -> Result<(), sqlx::Error> {
    sqlx::query_file!("queries/topic_set_title.sql", id, title)
        .execute(db)
        .await?;
    Ok(())
}

/// 交易帖的剩余库存（仍有效的公开报价的可成交次数减去已成交订单）：`(交易帖 ID, 库存)`；没有有效公开报价的不返回。
pub async fn listing_stock(
    db: impl PgExecutor<'_>,
    listing_ids: &[i64],
) -> Result<Vec<(i64, i64)>, sqlx::Error> {
    if listing_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query_file!("queries/listing_stock.sql", listing_ids)
        .fetch_all(db)
        .await?;
    Ok(rows.into_iter().map(|r| (r.listing_id, r.stock)).collect())
}

/// 指定楼层号的发帖时间 `(楼层, 时间)`，按楼层升序；不存在的楼层不返回。
pub async fn floor_times(
    db: impl PgExecutor<'_>,
    topic_id: i64,
    floors: &[i32],
) -> Result<Vec<(i32, OffsetDateTime)>, sqlx::Error> {
    let rows = sqlx::query_file!("queries/topic_timeline.sql", topic_id, floors)
        .fetch_all(db)
        .await?;
    Ok(rows.into_iter().map(|r| (r.floor, r.created_at)).collect())
}

/// 批量累加浏览数（worker 每 10 秒回写一次）。
pub async fn add_views(
    db: impl PgExecutor<'_>,
    ids: &[i64],
    counts: &[i64],
) -> Result<u64, sqlx::Error> {
    let r = sqlx::query_file!("queries/topic_add_views.sql", ids, counts)
        .execute(db)
        .await?;
    Ok(r.rows_affected())
}
