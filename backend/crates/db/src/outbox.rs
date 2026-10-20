//! 事务发件箱：业务写入与「待发送」同一事务；worker 用 `SKIP LOCKED` 认领（带租期）后逐条投递、逐条标记。
//!
//! 一条消息的状态：待发（`sent_at`、`failed_at` 都为空；`attempts > 0` 表示投递中或退避中）→ 已发（`sent_at`，
//! payload 清空）或死信（`failed_at`，保留 payload 供站长重新投递）→ 丢弃（`discarded_at`，payload 清空）。

use serde_json::Value;
use sqlx::PgExecutor;
use time::OffsetDateTime;

/// 验证码等需要尽快送达的消息。
pub const PRIORITY_HIGH: i16 = 10;
pub const PRIORITY_NORMAL: i16 = 0;
/// 尝试这么多次仍失败就进死信。退避 2^n 秒乘 0.5–1 的随机系数（最长 1 小时），用尽约 2.5–5 小时，
/// 扛得住邮件服务商的短时故障。
pub const MAX_ATTEMPTS: i32 = 16;
/// 认领的租期：投递者在这段时间内没有标记（崩溃、卡住），消息重新到期，由任一 worker 再次认领。
pub const LEASE_SECS: f64 = 1200.0;
/// 死信保留 14 天（内含邮箱地址等），之后自动丢弃、清空 payload。
pub const DEAD_LETTER_RETENTION_SECS: f64 = 14.0 * 86_400.0;

/// 认领到的一条消息。`attempts` 含这一次，同时是这次认领的令牌（标记时核对）。
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

/// 推送到管理员 Telegram 群的告警（ADR 0013 第 6 条）。
pub const KIND_STAFF_ALERT: &str = "staff_alert";

/// 管理员群告警：管理人员新设备登录、异常管理操作、仲裁裁决、代付余额不足、indexer 节点切换等都经这里入队，
/// 由 worker 推送。`event` 是简短的类型标识（如 `staff.new_device`），`text` 是给人看的正文（纯文本，不解析 Markdown）。
pub async fn staff_alert(
    db: impl PgExecutor<'_>,
    event: &str,
    text: &str,
) -> Result<i64, sqlx::Error> {
    insert_staff_alert(db, event, text, None)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

/// 同 [`staff_alert`]，但 `dedupe_secs` 内已有同一 `event` 的告警时不再写入（持续存在的状况，如余额不足、
/// 节点切换，不必每次巡检都推送）。返回是否写入。并发写入可能偶尔多发一条，不会漏发。
pub async fn staff_alert_deduped(
    db: impl PgExecutor<'_>,
    event: &str,
    text: &str,
    dedupe_secs: f64,
) -> Result<bool, sqlx::Error> {
    Ok(insert_staff_alert(db, event, text, Some(dedupe_secs))
        .await?
        .is_some())
}

async fn insert_staff_alert(
    db: impl PgExecutor<'_>,
    event: &str,
    text: &str,
    dedupe_secs: Option<f64>,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"INSERT INTO outbox (kind, payload, priority, dedupe_key)
           SELECT $1, $2, $3, $4
           WHERE $5::float8 IS NULL OR NOT EXISTS (
             SELECT 1 FROM outbox WHERE dedupe_key = $4 AND created_at > now() - make_interval(secs => $5))
           RETURNING id AS "id!""#,
        KIND_STAFF_ALERT,
        serde_json::json!({ "event": event, "text": text }),
        PRIORITY_HIGH,
        event,
        dedupe_secs
    )
    .fetch_optional(db)
    .await
}

/// 认领一批到期的待发消息（见 `queries/outbox_claim.sql`），按优先级、ID 排序。认领立即生效（不持有事务）。
pub async fn claim(
    db: impl PgExecutor<'_>,
    limit: i64,
    lease_secs: f64,
) -> Result<Vec<Message>, sqlx::Error> {
    sqlx::query_file_as!(Message, "queries/outbox_claim.sql", limit, lease_secs)
        .fetch_all(db)
        .await
}

/// 标记已发送并清空 payload。返回 `false` 表示这次认领已失效（租期过后被重新认领，或已被站长处理）：
/// 调用方应回滚同一事务里的投递。
pub async fn mark_sent(
    db: impl PgExecutor<'_>,
    id: i64,
    attempts: i32,
) -> Result<bool, sqlx::Error> {
    let r = sqlx::query!(
        "UPDATE outbox SET sent_at = now(), payload = '{}'
         WHERE id = $1 AND attempts = $2 AND sent_at IS NULL AND failed_at IS NULL",
        id,
        attempts
    )
    .execute(db)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// 记录一次失败。`permanent`（重试也不会成功）或尝试次数用尽时进死信，否则指数退避（2^attempts 秒，最长 1 小时，
/// 乘 0.5–1 的随机系数，避免同时失败的消息同时重试）。`error` 由调用方去掉敏感内容，这里截到 500 字。
/// 返回是否进了死信；这次认领已失效时返回 `None`（什么都没改）。
pub async fn mark_failed(
    db: impl PgExecutor<'_>,
    id: i64,
    attempts: i32,
    error: &str,
    permanent: bool,
) -> Result<Option<bool>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"UPDATE outbox SET
             last_error = left($3, 500),
             failed_at = CASE WHEN $4 OR attempts >= $5 THEN now() END,
             next_attempt_at = now() + make_interval(secs => least(power(2, attempts), 3600) * (0.5 + random() / 2))
           WHERE id = $1 AND attempts = $2 AND sent_at IS NULL AND failed_at IS NULL
           RETURNING failed_at IS NOT NULL AS "dead!""#,
        id,
        attempts,
        error,
        permanent,
        MAX_ATTEMPTS
    )
    .fetch_optional(db)
    .await
}

/// 每种消息的待发与死信条数（日志与后台概览；不读 payload）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindCounts {
    pub kind: String,
    /// 待发（含退避中与投递中）。
    pub pending: i64,
    /// 其中失败过、正在退避（或投递中）的。
    pub retrying: i64,
    /// 死信（未丢弃）。
    pub dead: i64,
    /// 最早一条待发消息的创建时间。
    pub oldest_pending: Option<OffsetDateTime>,
}

pub async fn counts(db: impl PgExecutor<'_>) -> Result<Vec<KindCounts>, sqlx::Error> {
    sqlx::query_file_as!(KindCounts, "queries/outbox_counts.sql")
        .fetch_all(db)
        .await
}

/// 后台死信列表的一行（不含 payload）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    pub id: i64,
    pub kind: String,
    pub attempts: i32,
    pub last_error: Option<String>,
    pub created_at: OffsetDateTime,
    pub failed_at: OffsetDateTime,
}

/// 未丢弃的死信，新的在前；`kind` 只看这一种，`before` 为上一页最后一条的 ID。
pub async fn dead_letters(
    db: impl PgExecutor<'_>,
    kind: Option<&str>,
    before: Option<i64>,
    limit: i64,
) -> Result<Vec<DeadLetter>, sqlx::Error> {
    sqlx::query_file_as!(
        DeadLetter,
        "queries/outbox_dead_letters.sql",
        kind,
        before,
        limit
    )
    .fetch_all(db)
    .await
}

/// 被重新投递或丢弃的死信（按 ID 排序）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handled {
    pub id: i64,
    pub kind: String,
}

/// 把死信放回待发队列（尝试次数清零、立即到期；`last_error` 留着，下次失败时覆盖）。不是死信的 ID 忽略。
pub async fn requeue(db: impl PgExecutor<'_>, ids: &[i64]) -> Result<Vec<Handled>, sqlx::Error> {
    sqlx::query_as!(
        Handled,
        r#"WITH h AS (
             UPDATE outbox SET failed_at = NULL, attempts = 0, next_attempt_at = now()
             WHERE id = ANY($1) AND failed_at IS NOT NULL AND discarded_at IS NULL
             RETURNING id, kind)
           SELECT id AS "id!", kind AS "kind!" FROM h ORDER BY id"#,
        ids
    )
    .fetch_all(db)
    .await
}

/// 丢弃死信并清空 payload。不是死信的 ID 忽略。
pub async fn discard(db: impl PgExecutor<'_>, ids: &[i64]) -> Result<Vec<Handled>, sqlx::Error> {
    sqlx::query_as!(
        Handled,
        r#"WITH h AS (
             UPDATE outbox SET discarded_at = now(), payload = '{}'
             WHERE id = ANY($1) AND failed_at IS NOT NULL AND discarded_at IS NULL
             RETURNING id, kind)
           SELECT id AS "id!", kind AS "kind!" FROM h ORDER BY id"#,
        ids
    )
    .fetch_all(db)
    .await
}

/// 丢弃进死信超过 `older_than_secs` 的消息（[`DEAD_LETTER_RETENTION_SECS`]），返回条数。
pub async fn discard_expired(
    db: impl PgExecutor<'_>,
    older_than_secs: f64,
) -> Result<u64, sqlx::Error> {
    let r = sqlx::query_file!("queries/outbox_dead_expire.sql", older_than_secs)
        .execute(db)
        .await?;
    Ok(r.rows_affected())
}
