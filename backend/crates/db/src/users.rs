//! 账号与登录设备。

use sqlx::{PgExecutor, PgPool};
use time::OffsetDateTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: i64,
    pub email: String,
    pub username: Option<String>,
    pub created_at: OffsetDateTime,
}

/// 按归一化邮箱取用户，不存在则创建。返回 `(用户, 是否新建)`。
pub async fn upsert_by_email(
    db: impl PgExecutor<'_>,
    email: &str,
) -> Result<(User, bool), sqlx::Error> {
    // xmax = 0 表示本语句插入的新行；冲突时 DO UPDATE 让 RETURNING 也能拿到已有行。
    let row = sqlx::query!(
        r#"INSERT INTO users (email) VALUES ($1)
           ON CONFLICT (email) DO UPDATE SET email = EXCLUDED.email
           RETURNING id, email, username, created_at, (xmax = 0) AS "created!""#,
        email
    )
    .fetch_one(db)
    .await?;
    Ok((
        User {
            id: row.id,
            email: row.email,
            username: row.username,
            created_at: row.created_at,
        },
        row.created,
    ))
}

pub async fn find_by_id(db: &PgPool, id: i64) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as!(
        User,
        "SELECT id, email, username, created_at FROM users WHERE id = $1",
        id
    )
    .fetch_optional(db)
    .await
}

/// 记录一次登录设备，返回该设备是否第一次出现。
pub async fn touch_device(
    db: impl PgExecutor<'_>,
    user_id: i64,
    device_hash: &[u8],
) -> Result<bool, sqlx::Error> {
    let row = sqlx::query!(
        r#"INSERT INTO user_devices (user_id, device_hash) VALUES ($1, $2)
           ON CONFLICT (user_id, device_hash) DO UPDATE SET last_seen_at = now()
           RETURNING (xmax = 0) AS "new!""#,
        user_id,
        device_hash
    )
    .fetch_one(db)
    .await?;
    Ok(row.new)
}

/// 发帖者自身的信息：注册时间（新用户限制）与等级（板块权限）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Actor {
    pub id: i64,
    pub created_at: OffsetDateTime,
    pub level: i16,
}

pub async fn actor(db: impl PgExecutor<'_>, id: i64) -> Result<Option<Actor>, sqlx::Error> {
    sqlx::query_file_as!(Actor, "queries/users_actor.sql", id)
        .fetch_optional(db)
        .await
}

/// 交易信誉计数（plan 第 7 节；只来自最终确认的订单，见 `reputation`）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TradeCounts {
    pub success: i32,
    pub seller_fault_fail: i32,
    pub buyer_fault_fail: i32,
    pub disputes_lost: i32,
    pub settled: i32,
}

/// 公开的拍卖违约记录（plan 第 6 节「赢家违约」：违约记录在信誉里公开显示）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AuctionRecord {
    /// 近 12 个月的违约次数。
    pub defaults: i32,
    /// 违约自动禁拍仍在生效时的结束时间。只看违约带来的禁拍：管理人员手动的处罚不公开。
    pub banned_until: Option<OffsetDateTime>,
}

/// 用户名片（列表页批量取）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub id: i64,
    pub username: Option<String>,
    pub avatar_url: Option<String>,
    pub level: i16,
    pub trade: TradeCounts,
    pub auction: AuctionRecord,
}

pub async fn cards(db: impl PgExecutor<'_>, ids: &[i64]) -> Result<Vec<Card>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query_file!("queries/users_cards.sql", ids)
        .fetch_all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| Card {
            id: r.id,
            username: r.username,
            avatar_url: r.avatar_url,
            level: r.level,
            trade: TradeCounts {
                success: r.success,
                seller_fault_fail: r.seller_fault_fail,
                buyer_fault_fail: r.buyer_fault_fail,
                disputes_lost: r.disputes_lost,
                settled: r.settled,
            },
            auction: AuctionRecord {
                defaults: r.auction_defaults,
                banned_until: r.auction_banned_until,
            },
        })
        .collect())
}

/// 个人主页。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub id: i64,
    pub username: Option<String>,
    pub avatar_url: Option<String>,
    pub created_at: OffsetDateTime,
    pub last_seen_at: Option<OffsetDateTime>,
    pub show_online: bool,
    pub level: i16,
    pub exp: i64,
    pub trade: TradeCounts,
    pub auction: AuctionRecord,
    /// 成功订单金额合计（USDT 最小单位，十进制字符串）。
    pub volume: String,
    pub avg_reply_secs: Option<i32>,
}

pub async fn profile(db: impl PgExecutor<'_>, id: i64) -> Result<Option<Profile>, sqlx::Error> {
    let row = sqlx::query_file!("queries/users_profile.sql", id)
        .fetch_optional(db)
        .await?;
    Ok(row.map(|r| Profile {
        id: r.id,
        username: r.username,
        avatar_url: r.avatar_url,
        created_at: r.created_at,
        last_seen_at: r.last_seen_at,
        show_online: r.show_online,
        level: r.level,
        exp: r.exp,
        trade: TradeCounts {
            success: r.success,
            seller_fault_fail: r.seller_fault_fail,
            buyer_fault_fail: r.buyer_fault_fail,
            disputes_lost: r.disputes_lost,
            settled: r.settled,
        },
        auction: AuctionRecord {
            defaults: r.auction_defaults,
            banned_until: r.auction_banned_until,
        },
        volume: r.volume,
        avg_reply_secs: r.avg_reply_secs,
    }))
}

/// 用户名（不区分大小写）对应的用户 ID。
pub async fn id_by_name(db: impl PgExecutor<'_>, name: &str) -> Result<Option<i64>, sqlx::Error> {
    let row = sqlx::query_file!("queries/users_id_by_name.sql", name)
        .fetch_optional(db)
        .await?;
    Ok(row.map(|r| r.id))
}

/// 一批用户名（已转小写）对应的用户 ID；不存在的跳过。
pub async fn ids_by_names(
    db: impl PgExecutor<'_>,
    lower_names: &[String],
) -> Result<Vec<i64>, sqlx::Error> {
    if lower_names.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query_file!("queries/users_ids_by_names.sql", lower_names)
        .fetch_all(db)
        .await?;
    Ok(rows.into_iter().map(|r| r.id).collect())
}

/// 设置用户名的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetUsername {
    Done,
    /// 30 天内已改过。
    TooSoon,
    /// 已被别人使用（不区分大小写）。
    Taken,
}

pub async fn set_username(
    db: impl PgExecutor<'_>,
    id: i64,
    name: &str,
) -> Result<SetUsername, sqlx::Error> {
    match sqlx::query_file!("queries/username_set.sql", id, name)
        .fetch_optional(db)
        .await
    {
        Ok(Some(_)) => Ok(SetUsername::Done),
        Ok(None) => Ok(SetUsername::TooSoon),
        Err(sqlx::Error::Database(e)) if e.constraint() == Some("users_username") => {
            Ok(SetUsername::Taken)
        }
        Err(e) => Err(e),
    }
}

pub async fn set_avatar(db: impl PgExecutor<'_>, id: i64, url: &str) -> Result<(), sqlx::Error> {
    sqlx::query!("UPDATE users SET avatar_url = $2 WHERE id = $1", id, url)
        .execute(db)
        .await?;
    Ok(())
}

/// @提及候选：用户名以 `prefix` 开头（不区分大小写）的用户，按用户名排序。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mention {
    pub id: i64,
    pub username: String,
    pub avatar_url: Option<String>,
    pub level: i16,
}

pub async fn search_prefix(
    db: impl PgExecutor<'_>,
    prefix: &str,
    limit: i64,
) -> Result<Vec<Mention>, sqlx::Error> {
    sqlx::query_file_as!(Mention, "queries/users_search.sql", prefix, limit)
        .fetch_all(db)
        .await
}

/// 用户主页的一条帖子。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserPost {
    pub id: i64,
    pub topic_id: i64,
    pub floor: i32,
    /// 纯文本前 200 字。
    pub excerpt: String,
    pub created_at: OffsetDateTime,
    pub title: String,
    pub forum_id: i32,
}

/// 用户的帖子（游标：上一页最后一个帖子 ID），只含 `forum_ids` 里的板块。
pub async fn posts(
    db: impl PgExecutor<'_>,
    author_id: i64,
    before_id: i64,
    forum_ids: &[i32],
    limit: i64,
) -> Result<Vec<UserPost>, sqlx::Error> {
    sqlx::query_file_as!(
        UserPost,
        "queries/user_posts.sql",
        author_id,
        before_id,
        forum_ids,
        limit
    )
    .fetch_all(db)
    .await
}

/// `(主题数, 帖子数)`，均不含已删除的。
pub async fn post_counts(
    db: impl PgExecutor<'_>,
    author_id: i64,
) -> Result<(i64, i64), sqlx::Error> {
    let r = sqlx::query_file!("queries/user_post_counts.sql", author_id)
        .fetch_one(db)
        .await?;
    Ok((r.topics, r.posts))
}

/// 在线状态需要的资料：是否公开在线状态、最近活跃时间。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Visibility {
    pub id: i64,
    pub show_online: bool,
    pub last_seen_at: Option<OffsetDateTime>,
}

pub async fn visibility(
    db: impl PgExecutor<'_>,
    ids: &[i64],
) -> Result<Vec<Visibility>, sqlx::Error> {
    sqlx::query_as!(
        Visibility,
        "SELECT id, show_online, last_seen_at FROM users WHERE id = ANY($1::bigint[])",
        ids
    )
    .fetch_all(db)
    .await
}

/// 记录最近活跃时间，一分钟内最多写一次。
pub async fn touch_last_seen(db: impl PgExecutor<'_>, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE users SET last_seen_at = now()
         WHERE id = $1 AND (last_seen_at IS NULL OR last_seen_at < now() - interval '1 minute')",
        id
    )
    .execute(db)
    .await?;
    Ok(())
}
