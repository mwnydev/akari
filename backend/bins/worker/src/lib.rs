//! 发件箱投递、浏览数回写、报价卡片过期。
//!
//! 现在是最小实现：轮询 `outbox`，`FOR UPDATE SKIP LOCKED` 锁一批、投递、标记，失败按指数退避重试。
//! 发薪、超时处理等关键任务按 plan 第 2 节接入 apalis（PostgreSQL 队列）时，发件箱改为向 apalis 入队，这里只保留转发。

use forum_db::{PgPool, outbox};
use forum_kv::ForumCache;
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

/// 一条发件箱消息的投递方式。
#[derive(Debug)]
pub enum Delivery {
    Email(Email),
    /// 站内通知（回复、@）。通知表与 WebSocket 推送在通知功能里实现；在此之前只标记为已处理，不重试。
    InSite,
}

/// 把发件箱消息渲染成投递动作。新增消息类型时在这里加分支。
pub fn render(msg: &outbox::Message) -> Result<Delivery, RenderError> {
    if msg.kind.starts_with("notify.") {
        return Ok(Delivery::InSite);
    }
    render_email(msg).map(Delivery::Email)
}

fn render_email(msg: &outbox::Message) -> Result<Email, RenderError> {
    let key = format!("outbox-{}", msg.id);
    let (to, (subject, html, text)) = match msg.kind.as_str() {
        "email.login_code" => {
            let p: LoginCode = serde_json::from_value(msg.payload.clone())?;
            (p.to, templates::login_code(&p.code))
        }
        "email.wallet_code" => {
            let p: LoginCode = serde_json::from_value(msg.payload.clone())?;
            (p.to, templates::wallet_code(&p.code))
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
            Ok(Delivery::Email(email)) => mailer.send(&email).await.map_err(|e| e.to_string()),
            Ok(Delivery::InSite) => Ok(()),
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

/// 报价卡片过期检查间隔。卡片过期时间到了之后，接口读取时已按「已过期」展示，这里只是落库。
pub const OFFER_EXPIRY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);
/// 每批最多标记的过期卡片数。
pub const OFFER_EXPIRY_BATCH: i64 = 500;

/// 把到期的待确认报价卡片标为已过期（plan 第 7 节），返回总条数。一批满了就继续。
pub async fn expire_offers(db: &PgPool) -> Result<u64, sqlx::Error> {
    let mut total = 0;
    loop {
        let n = forum_db::trade::expire_due(db, OFFER_EXPIRY_BATCH).await?;
        total += n;
        if n < OFFER_EXPIRY_BATCH as u64 {
            return Ok(total);
        }
    }
}

/// 浏览数回写间隔（plan 第 8 节）。
pub const VIEW_FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Debug, thiserror::Error)]
pub enum FlushError {
    #[error(transparent)]
    Kv(#[from] forum_kv::KvError),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// 把 Valkey 里累积的浏览数批量写入 `topics.view_count`，返回涉及的主题数。
/// 写库失败时把取走的计数加回 Valkey，下次再写。
pub async fn flush_views(db: &PgPool, cache: &ForumCache) -> Result<usize, FlushError> {
    let views = cache.take_views().await?;
    if views.is_empty() {
        return Ok(0);
    }
    let (ids, counts): (Vec<i64>, Vec<i64>) = views.iter().copied().unzip();
    if let Err(e) = forum_db::topics::add_views(db, &ids, &counts).await {
        cache.restore_views(&views).await?;
        return Err(e.into());
    }
    Ok(views.len())
}
