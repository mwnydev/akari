//! 事务发件箱：业务写入与「待发送」同一事务；worker 用 `SKIP LOCKED` 并发安全地取出投递。

use serde_json::Value;
use sqlx::{PgExecutor, Postgres, Transaction};

/// 验证码等需要尽快送达的消息。
pub const PRIORITY_HIGH: i16 = 10;
pub const PRIORITY_NORMAL: i16 = 0;
/// 超过这个次数放弃投递并记 `failed_at`。
pub const MAX_ATTEMPTS: i32 = 10;

#[derive(Debug, Clone)]
pub struct Message {
    pub id: i64,
    pub kind: String,
    pub payload: Value,
    pub attempts: i32,
}

pub async fn enqueue(
    db: impl PgExecutor<'_>,
    kind: &str,
    payload: &Value,
    priority: i16,
) -> Result<i64, sqlx::Error> {
    let row = sqlx::query!(
        "INSERT INTO outbox (kind, payload, priority) VALUES ($1, $2, $3) RETURNING id",
        kind,
        payload,
        priority
    )
    .fetch_one(db)
    .await?;
    Ok(row.id)
}

/// 在事务内锁定一批到期的待发消息；其他 worker 会跳过这些行。
pub async fn lock_due(
    tx: &mut Transaction<'_, Postgres>,
    limit: i64,
) -> Result<Vec<Message>, sqlx::Error> {
    sqlx::query_as!(
        Message,
        "SELECT id, kind, payload, attempts FROM outbox
         WHERE sent_at IS NULL AND failed_at IS NULL AND next_attempt_at <= now()
         ORDER BY priority DESC, id
         LIMIT $1
         FOR UPDATE SKIP LOCKED",
        limit
    )
    .fetch_all(&mut **tx)
    .await
}

/// 标记已发送并清空 payload。
pub async fn mark_sent(db: impl PgExecutor<'_>, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE outbox SET sent_at = now(), payload = '{}' WHERE id = $1",
        id
    )
    .execute(db)
    .await?;
    Ok(())
}

/// 记录失败：指数退避（2^attempts 秒，最长 1 小时）；达到上限则放弃并清空 payload。
pub async fn mark_failed(db: impl PgExecutor<'_>, id: i64, error: &str) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE outbox SET
           attempts = attempts + 1,
           last_error = left($2, 500),
           next_attempt_at = now() + make_interval(secs => least(power(2, attempts + 1), 3600)),
           failed_at = CASE WHEN attempts + 1 >= $3 THEN now() END,
           payload = CASE WHEN attempts + 1 >= $3 THEN '{}'::jsonb ELSE payload END
         WHERE id = $1",
        id,
        error,
        MAX_ATTEMPTS
    )
    .execute(db)
    .await?;
    Ok(())
}
