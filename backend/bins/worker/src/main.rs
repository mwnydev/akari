//! forum-worker：后台任务：发件箱投递、浏览数回写、报价卡片过期；之后接入 apalis 队列（通知、超时处理、发薪）与图片转换。

use std::time::Duration;

use anyhow::{Context, bail};
use forum_mail::{AnyMailer, Failover, ResendMailer, SmtpMailer};

const POLL: Duration = Duration::from_millis(500);

fn env(name: &str) -> anyhow::Result<String> {
    std::env::var(name).with_context(|| format!("missing env {name}"))
}

fn build_mailer(kind: &str, from: &str) -> anyhow::Result<AnyMailer> {
    Ok(match kind {
        "resend" => AnyMailer::Resend(ResendMailer::new(&env("RESEND_API_KEY")?, from)),
        "smtp" => AnyMailer::Smtp(SmtpMailer::from_url(&env("SMTP_URL")?, from)?),
        other => bail!("unknown mail channel {other}"),
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    forum_runtime::init_tracing();
    forum_runtime::install_crypto_provider();
    let shutdown = forum_runtime::shutdown_token();

    let db = forum_db::connect(&env("DATABASE_URL")?, 10).await?;
    let cache = forum_kv::ForumCache::new(forum_kv::pool(&env("VALKEY_URL")?)?, "");
    let from = env("MAIL_FROM")?;
    // 主备通道可在配置里一键互换（plan 第 9 节）：MAIL_PRIMARY=resend MAIL_BACKUP=smtp。
    let primary = build_mailer(
        &std::env::var("MAIL_PRIMARY").unwrap_or_else(|_| "smtp".into()),
        &from,
    )?;
    let backup = match std::env::var("MAIL_BACKUP") {
        Ok(kind) if !kind.is_empty() => Some(build_mailer(&kind, &from)?),
        _ => None,
    };
    let mailer = Failover::new(primary, backup);
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "forum-worker started");

    let views = tokio::spawn(view_flusher(db.clone(), cache, shutdown.clone()));
    let offers = tokio::spawn(offer_expirer(db.clone(), shutdown.clone()));
    loop {
        match forum_worker::drain_once(&db, &mailer).await {
            // 本批满了说明还有积压，立刻继续。
            Ok(n) if n >= forum_worker::BATCH => continue,
            Ok(_) => {}
            Err(e) => tracing::error!(error = %e, "outbox drain failed"),
        }
        tokio::select! {
            () = shutdown.cancelled() => break,
            () = tokio::time::sleep(POLL) => {}
        }
    }
    if let Err(e) = views.await {
        tracing::error!(error = %e, "view flusher panicked");
    }
    if let Err(e) = offers.await {
        tracing::error!(error = %e, "offer expirer panicked");
    }
    tracing::info!("forum-worker stopped");
    Ok(())
}

/// 每 10 秒回写浏览数；退出前再写一次。
async fn view_flusher(
    db: forum_db::PgPool,
    cache: forum_kv::ForumCache,
    shutdown: tokio_util::sync::CancellationToken,
) {
    let mut tick = tokio::time::interval(forum_worker::VIEW_FLUSH_INTERVAL);
    loop {
        let stopping = tokio::select! {
            () = shutdown.cancelled() => true,
            _ = tick.tick() => false,
        };
        if let Err(e) = forum_worker::flush_views(&db, &cache).await {
            tracing::error!(error = %e, "view flush failed");
        }
        if stopping {
            break;
        }
    }
}

/// 每分钟把到期的待确认报价卡片标为已过期。
async fn offer_expirer(db: forum_db::PgPool, shutdown: tokio_util::sync::CancellationToken) {
    let mut tick = tokio::time::interval(forum_worker::OFFER_EXPIRY_INTERVAL);
    loop {
        tokio::select! {
            () = shutdown.cancelled() => break,
            _ = tick.tick() => {}
        }
        match forum_worker::expire_offers(&db).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(count = n, "offers expired"),
            Err(e) => tracing::error!(error = %e, "offer expiry failed"),
        }
    }
}
