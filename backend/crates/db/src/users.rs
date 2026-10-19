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
