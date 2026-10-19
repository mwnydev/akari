//! 只增审计日志：只提供写入，数据库触发器拒绝 UPDATE / DELETE / TRUNCATE。

use serde_json::Value;
use sqlx::PgExecutor;

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
