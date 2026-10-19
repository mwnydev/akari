//! 发件箱投递。
//!
//! 现在是最小实现：轮询 `outbox`，`FOR UPDATE SKIP LOCKED` 锁一批、投递、标记，失败按指数退避重试。
//! 发薪、超时处理等关键任务按 plan 第 2 节接入 apalis（PostgreSQL 队列）时，发件箱改为向 apalis 入队，这里只保留转发。

use forum_db::{PgPool, outbox};
use forum_mail::{Email, Mailer, templates};
use serde::Deserialize;

/// 每批最多处理条数。投递期间持有行锁，批不宜大。
pub const BATCH: usize = 10;

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("unknown outbox kind")]
    UnknownKind,
    #[error("bad payload: {0}")]
    Payload(#[from] serde_json::Error),
}

#[derive(Deserialize)]
struct LoginCode {
    to: String,
    code: String,
}

#[derive(Deserialize)]
struct NewDevice {
    to: String,
    at: String,
}

/// 把发件箱消息渲染成邮件。新增消息类型时在这里加分支。
pub fn render(msg: &outbox::Message) -> Result<Email, RenderError> {
    let key = format!("outbox-{}", msg.id);
    let (to, (subject, html, text)) = match msg.kind.as_str() {
        "email.login_code" => {
            let p: LoginCode = serde_json::from_value(msg.payload.clone())?;
            (p.to, templates::login_code(&p.code))
        }
        "email.new_device" => {
            let p: NewDevice = serde_json::from_value(msg.payload.clone())?;
            (p.to, templates::new_device(&p.at))
        }
        _ => return Err(RenderError::UnknownKind),
    };
    Ok(Email {
        to,
        subject,
        html,
        text,
        idempotency_key: key,
    })
}

/// 处理一批到期消息，返回本批条数（0 表示暂时没有可发的）。
pub async fn drain_once<M: Mailer>(db: &PgPool, mailer: &M) -> Result<usize, sqlx::Error> {
    let mut tx = db.begin().await?;
    let batch = outbox::lock_due(&mut tx, BATCH as i64).await?;
    for msg in &batch {
        let result = match render(msg) {
            Ok(email) => mailer.send(&email).await.map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        };
        match result {
            Ok(()) => outbox::mark_sent(&mut *tx, msg.id).await?,
            Err(err) => {
                // 只记 ID、类型和错误，不记收件人与内容。
                tracing::warn!(id = msg.id, kind = %msg.kind, attempts = msg.attempts + 1, error = %err, "outbox delivery failed");
                outbox::mark_failed(&mut *tx, msg.id, &err).await?;
            }
        }
    }
    tx.commit().await?;
    Ok(batch.len())
}
