//! 只增审计日志：只提供写入与查询，数据库触发器拒绝 UPDATE / DELETE / TRUNCATE。
//! 撤销一条管理操作时另写一条 `reverts` 指向它的日志（每条最多撤销一次）。

use serde_json::Value;
use sqlx::PgExecutor;
use time::OffsetDateTime;

pub async fn append(
    db: impl PgExecutor<'_>,
    actor_id: i64,
    action: &str,
    target: Option<&str>,
    reason: Option<&str>,
    detail: &Value,
) -> Result<i64, sqlx::Error> {
    let row = sqlx::query!(
        "INSERT INTO admin_audit_logs (actor_id, action, target, reason, detail)
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
        actor_id,
        action,
        target,
        reason,
        detail
    )
    .fetch_one(db)
    .await?;
    Ok(row.id)
}

/// 写一条撤销记录；那条日志已经被撤销过返回 `None`。
pub async fn append_revert(
    db: impl PgExecutor<'_>,
    actor_id: i64,
    entry: &Entry,
    reason: &str,
    detail: &Value,
) -> Result<Option<i64>, sqlx::Error> {
    let action = format!("{}.revert", entry.action);
    let row = sqlx::query!(
        "INSERT INTO admin_audit_logs (actor_id, action, target, reason, detail, reverts)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (reverts) WHERE reverts IS NOT NULL DO NOTHING RETURNING id",
        actor_id,
        action,
        entry.target,
        reason,
        detail,
        entry.id
    )
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| r.id))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: i64,
    pub actor_id: i64,
    pub action: String,
    pub target: Option<String>,
    pub reason: Option<String>,
    pub detail: Value,
    pub created_at: OffsetDateTime,
    /// 这条是撤销记录时，被撤销的日志 ID。
    pub reverts: Option<i64>,
    /// 已被撤销。
    pub reverted: bool,
}

/// 某账号在 `[from, to)` 内的管理操作（不含撤销记录），新的在前，最多 `limit` 条。
pub async fn by_actor(
    db: impl PgExecutor<'_>,
    actor_id: i64,
    from: OffsetDateTime,
    to: OffsetDateTime,
    limit: i64,
) -> Result<Vec<Entry>, sqlx::Error> {
    sqlx::query_file_as!(
        Entry,
        "queries/audit_by_actor.sql",
        actor_id,
        from,
        to,
        limit
    )
    .fetch_all(db)
    .await
}

/// 审计日志列表（后台），新的在前；可按操作者筛选。`before` 为游标（上一页最后一条的 ID）。
pub async fn list(
    db: impl PgExecutor<'_>,
    actor_id: Option<i64>,
    before: Option<i64>,
    limit: i64,
) -> Result<Vec<Entry>, sqlx::Error> {
    sqlx::query_as!(
        Entry,
        r#"SELECT a.id, a.actor_id, a.action, a.target, a.reason, a.detail, a.created_at, a.reverts,
                  EXISTS (SELECT 1 FROM admin_audit_logs r WHERE r.reverts = a.id) AS "reverted!"
           FROM admin_audit_logs a
           WHERE ($1::bigint IS NULL OR a.actor_id = $1) AND a.id < $2
           ORDER BY a.id DESC LIMIT $3"#,
        actor_id,
        before.unwrap_or(i64::MAX),
        limit
    )
    .fetch_all(db)
    .await
}

/// 某账号最近 `window_secs` 秒内 `actions` 的次数（异常检测）；站长恢复过其管理权限的，只算恢复之后的。
pub async fn count_recent(
    db: impl PgExecutor<'_>,
    actor_id: i64,
    actions: &[&str],
    window_secs: i64,
) -> Result<i64, sqlx::Error> {
    let actions: Vec<String> = actions.iter().map(|a| (*a).to_owned()).collect();
    let row = sqlx::query_file!(
        "queries/audit_count_recent.sql",
        actor_id,
        &actions,
        window_secs as f64
    )
    .fetch_one(db)
    .await?;
    Ok(row.count)
}

/// 事务级锁：同一操作者的管理操作串行执行，异常检测的计数不会被并发请求绕过。
pub async fn lock_actor(db: impl PgExecutor<'_>, actor_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "SELECT pg_advisory_xact_lock(hashtextextended('staff-actor', $1))",
        actor_id
    )
    .execute(db)
    .await?;
    Ok(())
}
