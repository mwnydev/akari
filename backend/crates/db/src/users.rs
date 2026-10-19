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

/// 用户名片（列表页批量取）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub id: i64,
    pub username: Option<String>,
    pub level: i16,
}

pub async fn cards(db: impl PgExecutor<'_>, ids: &[i64]) -> Result<Vec<Card>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_file_as!(Card, "queries/users_cards.sql", ids)
        .fetch_all(db)
        .await
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
