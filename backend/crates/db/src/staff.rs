//! 管理角色（ADR 0013）、管理权限暂停与站长后台 Passkey。权限规则在 `forum_core::staff`。

use forum_core::staff::{Role, StaffRoles};
use serde_json::Value;
use sqlx::PgExecutor;
use time::OffsetDateTime;

pub const ROLE_OWNER: i16 = Role::Owner.id();
pub const ROLE_ADMIN: i16 = Role::Admin.id();
pub const ROLE_MODERATOR: i16 = Role::Moderator.id();

/// `passkeys.purpose`：站长后台登录（RP ID 为后台中心域名，ADR 0012）。
pub const PURPOSE_ADMIN: i16 = 2;

/// 一个账号的全部管理角色与是否被暂停（每个管理请求一次查询）。
pub async fn roles(db: impl PgExecutor<'_>, user_id: i64) -> Result<StaffRoles, sqlx::Error> {
    let rows = sqlx::query_file!("queries/staff_roles_of.sql", user_id)
        .fetch_all(db)
        .await?;
    let suspended = rows.first().is_some_and(|r| r.suspended);
    Ok(StaffRoles::from_rows(
        rows.into_iter().filter_map(|r| Some((r.role?, r.forum_id))),
        suspended,
    ))
}

/// 授予角色。版主必须指定板块，其余角色不能指定；已有同样的角色返回 `None`。
pub async fn grant(
    db: impl PgExecutor<'_>,
    user_id: i64,
    role: i16,
    forum_id: Option<i32>,
    granted_by: Option<i64>,
) -> Result<Option<i64>, sqlx::Error> {
    let row = sqlx::query!(
        "INSERT INTO staff_roles (user_id, role, forum_id, granted_by)
         VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING RETURNING id",
        user_id,
        role,
        forum_id,
        granted_by
    )
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| r.id))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    pub id: i64,
    pub user_id: i64,
    pub username: Option<String>,
    pub role: i16,
    pub forum_id: Option<i32>,
    pub granted_by: Option<i64>,
    pub created_at: OffsetDateTime,
}

/// 全部任命（后台管理人员列表）。管理人员只有几十人，不分页。
pub async fn assignments(db: impl PgExecutor<'_>) -> Result<Vec<Assignment>, sqlx::Error> {
    sqlx::query_as!(
        Assignment,
        "SELECT s.id, s.user_id, u.username, s.role, s.forum_id, s.granted_by, s.created_at
         FROM staff_roles s JOIN users u ON u.id = s.user_id ORDER BY s.role, s.id"
    )
    .fetch_all(db)
    .await
}

/// 撤销一条任命（站长的任命不能经接口撤销），返回被删的行。
pub async fn revoke(db: impl PgExecutor<'_>, id: i64) -> Result<Option<Assignment>, sqlx::Error> {
    sqlx::query_as!(
        Assignment,
        r#"DELETE FROM staff_roles s USING users u
           WHERE s.id = $1 AND s.role <> 1 AND u.id = s.user_id
           RETURNING s.id, s.user_id, u.username, s.role, s.forum_id, s.granted_by, s.created_at"#,
        id
    )
    .fetch_optional(db)
    .await
}

/// 暂停管理权限；已在暂停中返回 `None`（不重复记录、不重复告警）。
pub async fn suspend(
    db: impl PgExecutor<'_>,
    user_id: i64,
    reason: &str,
    detail: &Value,
) -> Result<Option<i64>, sqlx::Error> {
    let row = sqlx::query!(
        "INSERT INTO staff_suspensions (user_id, reason, detail) VALUES ($1, $2, $3)
         ON CONFLICT (user_id) WHERE reinstated_at IS NULL DO NOTHING RETURNING id",
        user_id,
        reason,
        detail
    )
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| r.id))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Suspension {
    pub id: i64,
    pub user_id: i64,
    pub username: Option<String>,
    pub reason: String,
    pub detail: Value,
    pub created_at: OffsetDateTime,
}

/// 暂停中的账号。
pub async fn suspensions(db: impl PgExecutor<'_>) -> Result<Vec<Suspension>, sqlx::Error> {
    sqlx::query_as!(
        Suspension,
        "SELECT s.id, s.user_id, u.username, s.reason, s.detail, s.created_at
         FROM staff_suspensions s JOIN users u ON u.id = s.user_id
         WHERE s.reinstated_at IS NULL ORDER BY s.id"
    )
    .fetch_all(db)
    .await
}

/// 恢复管理权限，返回被恢复的账号；不在暂停中返回 `None`。
pub async fn reinstate(
    db: impl PgExecutor<'_>,
    id: i64,
    by: i64,
) -> Result<Option<i64>, sqlx::Error> {
    let row = sqlx::query!(
        "UPDATE staff_suspensions SET reinstated_at = now(), reinstated_by = $2
         WHERE id = $1 AND reinstated_at IS NULL RETURNING user_id",
        id,
        by
    )
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| r.user_id))
}

#[derive(Debug, Clone, PartialEq)]
pub struct AdminPasskey {
    pub id: i64,
    pub credential_id: Vec<u8>,
    /// webauthn-rs 序列化的凭证（含签名计数）。
    pub credential: Value,
    pub created_at: OffsetDateTime,
    pub last_used_at: Option<OffsetDateTime>,
}

pub async fn admin_passkeys(
    db: impl PgExecutor<'_>,
    user_id: i64,
) -> Result<Vec<AdminPasskey>, sqlx::Error> {
    sqlx::query_as!(
        AdminPasskey,
        "SELECT id, credential_id, credential, created_at, last_used_at
         FROM passkeys WHERE user_id = $1 AND purpose = $2 ORDER BY id",
        user_id,
        PURPOSE_ADMIN
    )
    .fetch_all(db)
    .await
}

/// 保存后台 Passkey；凭证 ID 已存在返回 `None`。
pub async fn insert_admin_passkey(
    db: impl PgExecutor<'_>,
    user_id: i64,
    credential_id: &[u8],
    credential: &Value,
) -> Result<Option<i64>, sqlx::Error> {
    let row = sqlx::query!(
        "INSERT INTO passkeys (user_id, purpose, credential_id, credential, last_used_at)
         VALUES ($1, $2, $3, $4, now())
         ON CONFLICT (credential_id) DO NOTHING RETURNING id",
        user_id,
        PURPOSE_ADMIN,
        credential_id,
        credential
    )
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| r.id))
}
