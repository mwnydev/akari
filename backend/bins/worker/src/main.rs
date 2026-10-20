//! forum-worker：后台任务：发件箱投递（邮件、站内通知、实时推送）与巡检、浏览数回写、报价卡片过期、通知分区维护、
//! 发帖经验、每日信誉校对、图片转换、仲裁分配与期限、热门排名、拍卖结算与提醒；之后接入 apalis 队列（超时处理、发薪）。

use std::time::Duration;

use anyhow::{Context, bail};
use forum_mail::{AnyMailer, Failover, PostmarkMailer, ResendMailer, SmtpMailer};

const POLL: Duration = Duration::from_millis(500);

fn env(name: &str) -> anyhow::Result<String> {
    std::env::var(name).with_context(|| format!("missing env {name}"))
}

/// 邮件通道：生产只用 HTTP API（`resend` / `postmark`）；`smtp` 只能连本机（开发 Mailpit），
/// SMTP 会在邮件头里留下源站 IP。`env` 读通道的密钥。
fn build_mailer(
    kind: &str,
    from: &str,
    env: impl Fn(&str) -> anyhow::Result<String>,
) -> anyhow::Result<AnyMailer> {
    Ok(match kind {
        "resend" => AnyMailer::Resend(ResendMailer::new(&env("RESEND_API_KEY")?, from)),
        "postmark" => {
            AnyMailer::Postmark(PostmarkMailer::new(&env("POSTMARK_SERVER_TOKEN")?, from)?)
        }
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
    let kv = forum_kv::pool(&env("VALKEY_URL")?)?;
    let ns = forum_kv::namespace_from_env()?;
    let cache = forum_kv::ForumCache::new(kv.clone(), ns.as_str());
    let guest = forum_kv::GuestCache::new(kv.clone(), ns.as_str());
    let realtime = forum_kv::Realtime::new(kv, ns);
    let from = env("MAIL_FROM")?;
    // 主备通道可在配置里一键互换（plan 第 9 节）：生产 MAIL_PRIMARY=resend MAIL_BACKUP=postmark。
    let primary = build_mailer(
        &std::env::var("MAIL_PRIMARY").unwrap_or_else(|_| "smtp".into()),
        &from,
        env,
    )?;
    let backup = match std::env::var("MAIL_BACKUP") {
        Ok(kind) if !kind.is_empty() => Some(build_mailer(&kind, &from, env)?),
        _ => None,
    };
    let mailer = Failover::new(primary, backup);
    let telegram = forum_worker::telegram::Telegram::from_env()?;
    if telegram.is_none() {
        tracing::warn!(
            "TELEGRAM_BOT_TOKEN / TELEGRAM_ADMIN_CHAT_ID not set: staff alerts disabled"
        );
    }
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "forum-worker started");

    let images = match forum_media::StorageConfig::from_env()? {
        Some(c) => {
            let storage = forum_media::Storage::new(&c)?;
            if std::env::var("MEDIA_S3_CREATE_BUCKET").is_ok_and(|v| v == "true") {
                storage.ensure_bucket().await?;
            }
            let worker = forum_worker::images::ImageWorker {
                db: db.clone(),
                storage: std::sync::Arc::new(storage),
                processor: std::sync::Arc::new(forum_media::RustProcessor),
            };
            Some(tokio::spawn(worker.run(shutdown.clone())))
        }
        None => {
            tracing::warn!("MEDIA_S3_* not set: image processing disabled");
            None
        }
    };
    let exp = tokio::spawn(periodic(
        "post exp",
        forum_worker::exp::INTERVAL,
        (db.clone(), cache.clone()),
        shutdown.clone(),
        |db, cache| async move { forum_worker::exp::award_posts(&db, &cache).await },
    ));
    let stats = tokio::spawn(periodic(
        "trade stats audit",
        forum_worker::stats::INTERVAL,
        (db.clone(), cache.clone()),
        shutdown.clone(),
        |db, cache| async move { forum_worker::stats::audit_trade_stats(&db, &cache).await },
    ));
    let arbitration_cfg = forum_worker::arbitration::Config::from_env()?;
    let arbitration = tokio::spawn(periodic(
        "arbitration",
        forum_worker::arbitration::INTERVAL,
        (db.clone(), cache.clone()),
        shutdown.clone(),
        move |db, _| async move { forum_worker::arbitration::run_once(&db, arbitration_cfg).await },
    ));
    let outbox_report = tokio::spawn(periodic(
        "outbox report",
        forum_worker::OUTBOX_REPORT_INTERVAL,
        (db.clone(), cache.clone()),
        shutdown.clone(),
        |db, _| async move { forum_worker::outbox_report(&db).await },
    ));
    let hot = tokio::spawn(periodic(
        "hot ranking",
        forum_worker::HOT_INTERVAL,
        (db.clone(), cache.clone()),
        shutdown.clone(),
        |db, _| async move { forum_db::hot::refresh(&db).await },
    ));
    let auctions = match forum_worker::auctions::Live::from_env()? {
        Some(live) => {
            let cfg = forum_worker::auctions::Config::from_env()?;
            let live = std::sync::Arc::new(live);
            let notify = std::sync::Arc::new(forum_worker::auctions::Notify {
                realtime: realtime.clone(),
                guest,
                cards: cache.clone(),
            });
            Some(tokio::spawn(periodic(
                "auctions",
                forum_worker::auctions::INTERVAL,
                (db.clone(), cache.clone()),
                shutdown.clone(),
                move |db, _| {
                    let (live, notify) = (live.clone(), notify.clone());
                    async move {
                        forum_worker::auctions::run_once(&db, live.as_ref(), cfg, Some(&notify))
                            .await
                    }
                },
            )))
        }
        None => {
            tracing::warn!(
                "CHAIN_CONFIG / SPONSOR_URL / SPONSOR_API_TOKEN not set: auction settlement disabled"
            );
            None
        }
    };
    let auction_reminders = tokio::spawn(periodic(
        "auction reminders",
        forum_worker::auctions::REMIND_INTERVAL,
        (db.clone(), cache.clone()),
        shutdown.clone(),
        |db, _| async move { forum_worker::auctions::remind_once(&db).await },
    ));
    let views = tokio::spawn(view_flusher(db.clone(), cache, shutdown.clone()));
    let offers = tokio::spawn(offer_expirer(db.clone(), shutdown.clone()));
    let partitions = tokio::spawn(partition_keeper(db.clone(), shutdown.clone()));
    loop {
        match forum_worker::drain_once(&db, &mailer, telegram.as_ref(), Some(&realtime)).await {
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
    for (name, task) in [
        ("partition keeper", partitions),
        ("post exp", exp),
        ("trade stats audit", stats),
        ("arbitration", arbitration),
        ("hot ranking", hot),
        ("outbox report", outbox_report),
        ("auction reminders", auction_reminders),
    ]
    .into_iter()
    .chain(images.map(|t| ("image worker", t)))
    .chain(auctions.map(|t| ("auctions", t)))
    {
        if let Err(e) = task.await {
            tracing::error!(error = %e, task = name, "task panicked");
        }
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

/// 启动时与之后每天维护一次通知分区。
async fn partition_keeper(db: forum_db::PgPool, shutdown: tokio_util::sync::CancellationToken) {
    let mut tick = tokio::time::interval(forum_worker::PARTITION_INTERVAL);
    loop {
        tokio::select! {
            () = shutdown.cancelled() => break,
            _ = tick.tick() => {}
        }
        match forum_db::notifications::maintain_partitions(&db).await {
            Ok((created, dropped)) if created.is_empty() && dropped.is_empty() => {}
            Ok((created, dropped)) => {
                tracing::info!(?created, ?dropped, "notification partitions maintained")
            }
            Err(e) => tracing::error!(error = %e, "notification partition maintenance failed"),
        }
    }
}

/// 启动后立即跑一次，之后每隔 `every` 跑一次，直到 `shutdown`。任务返回处理条数（只用于日志）。
async fn periodic<F, Fut>(
    name: &'static str,
    every: Duration,
    (db, cache): (forum_db::PgPool, forum_kv::ForumCache),
    shutdown: tokio_util::sync::CancellationToken,
    job: F,
) where
    F: Fn(forum_db::PgPool, forum_kv::ForumCache) -> Fut,
    Fut: std::future::Future<Output = Result<usize, sqlx::Error>>,
{
    let mut tick = tokio::time::interval(every);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = shutdown.cancelled() => break,
            _ = tick.tick() => {}
        }
        match job(db.clone(), cache.clone()).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(task = name, count = n, "periodic task done"),
            Err(e) => tracing::error!(task = name, error = %e, "periodic task failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use forum_mail::Mailer;

    fn env_of<'a>(
        vars: &'a [(&'static str, &'static str)],
    ) -> impl Fn(&str) -> anyhow::Result<String> + 'a {
        move |k| {
            vars.iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| (*v).to_owned())
                .with_context(|| format!("missing env {k}"))
        }
    }

    #[tokio::test]
    async fn mail_channels_by_name() {
        let vars = [
            ("RESEND_API_KEY", "re_test"),
            ("POSTMARK_SERVER_TOKEN", "pm-test-token"),
            ("SMTP_URL", "smtp://127.0.0.1:1025"),
        ];
        for kind in ["resend", "postmark", "smtp"] {
            let m = build_mailer(kind, "noreply@akari.test", env_of(&vars)).unwrap();
            assert_eq!(m.name(), kind);
        }
        assert!(build_mailer("sendgrid", "noreply@akari.test", env_of(&vars)).is_err());
        // 选了通道却没给密钥：拒绝启动
        let err = build_mailer("postmark", "noreply@akari.test", env_of(&[]))
            .err()
            .unwrap();
        assert!(err.to_string().contains("POSTMARK_SERVER_TOKEN"), "{err}");
    }
}
