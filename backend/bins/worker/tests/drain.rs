//! 需要 PostgreSQL（`DATABASE_URL`）。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

use std::sync::Mutex;

use forum_db::{PgPool, outbox};
use forum_mail::{Email, MailError, Mailer};
use serde_json::json;

#[derive(Default)]
struct Recorder {
    sent: Mutex<Vec<Email>>,
    fail: bool,
}

impl Mailer for Recorder {
    fn name(&self) -> &'static str {
        "recorder"
    }
    async fn send(&self, email: &Email) -> Result<(), MailError> {
        if self.fail {
            return Err(MailError::Transport("down".into()));
        }
        self.sent.lock().unwrap().push(email.clone());
        Ok(())
    }
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn delivers_login_code_first_and_marks_sent(pool: PgPool) {
    outbox::enqueue(
        &pool,
        "email.new_device",
        &json!({"to": "a@example.com", "at": "2026-10-19T00:00:00Z"}),
        0,
    )
    .await
    .unwrap();
    let code_id = outbox::enqueue(
        &pool,
        "email.login_code",
        &json!({"to": "b@example.com", "code": "123456"}),
        outbox::PRIORITY_HIGH,
    )
    .await
    .unwrap();

    let mailer = Recorder::default();
    assert_eq!(forum_worker::drain_once(&pool, &mailer).await.unwrap(), 2);
    assert_eq!(
        forum_worker::drain_once(&pool, &mailer).await.unwrap(),
        0,
        "nothing left"
    );

    let sent = mailer.sent.lock().unwrap();
    assert_eq!(sent[0].to, "b@example.com", "high priority first");
    assert!(sent[0].text.contains("123456"));
    assert_eq!(sent[0].idempotency_key, format!("outbox-{code_id}"));
    assert_eq!(sent[1].to, "a@example.com");
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn failures_are_retried_later(pool: PgPool) {
    let id = outbox::enqueue(
        &pool,
        "email.login_code",
        &json!({"to": "c@example.com", "code": "1"}),
        0,
    )
    .await
    .unwrap();
    let bad = outbox::enqueue(&pool, "no.such.kind", &json!({}), 0)
        .await
        .unwrap();
    let mailer = Recorder {
        fail: true,
        ..Default::default()
    };
    assert_eq!(forum_worker::drain_once(&pool, &mailer).await.unwrap(), 2);

    let rows: Vec<(i64, i32, Option<String>)> =
        sqlx::query_as("SELECT id, attempts, last_error FROM outbox ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(rows[0], (id, 1, Some("transport: down".into())));
    assert_eq!(rows[1].0, bad);
    assert_eq!(rows[1].2.as_deref(), Some("unknown outbox kind"));
    // 退避期内不再取出
    assert_eq!(forum_worker::drain_once(&pool, &mailer).await.unwrap(), 0);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn in_site_notifications_are_not_emailed_or_retried(pool: PgPool) {
    let id = outbox::enqueue(
        &pool,
        "notify.reply",
        &json!({"to": 1, "from": 2, "topic_id": 3, "post_id": 4, "floor": 2}),
        0,
    )
    .await
    .unwrap();
    let mailer = Recorder::default();
    assert_eq!(forum_worker::drain_once(&pool, &mailer).await.unwrap(), 1);
    assert!(mailer.sent.lock().unwrap().is_empty());
    let sent: Option<time::OffsetDateTime> =
        sqlx::query_scalar("SELECT sent_at FROM outbox WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(sent.is_some());
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn view_counts_flush_to_topics(pool: PgPool) {
    let Ok(url) = std::env::var("VALKEY_URL") else {
        eprintln!("VALKEY_URL not set, skipping valkey-backed test");
        return;
    };
    let cache = forum_kv::ForumCache::new(
        forum_kv::pool(&url).unwrap(),
        format!("wtest{}:", std::process::id()),
    );
    let topic: i64 = sqlx::query_scalar(
        "INSERT INTO topics (forum_id, author_id, title, last_post_at) VALUES (1, 1, 't', now()) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    for _ in 0..3 {
        cache.add_view(topic).await.unwrap();
    }
    assert_eq!(forum_worker::flush_views(&pool, &cache).await.unwrap(), 1);
    assert_eq!(forum_worker::flush_views(&pool, &cache).await.unwrap(), 0);
    let views: i64 = sqlx::query_scalar("SELECT view_count FROM topics WHERE id = $1")
        .bind(topic)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(views, 3);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn wallet_code_is_rendered(pool: PgPool) {
    outbox::enqueue(
        &pool,
        "email.wallet_code",
        &json!({"to": "c@example.com", "code": "654321"}),
        outbox::PRIORITY_HIGH,
    )
    .await
    .unwrap();
    let mailer = Recorder::default();
    assert_eq!(forum_worker::drain_once(&pool, &mailer).await.unwrap(), 1);
    let sent = mailer.sent.lock().unwrap();
    assert!(sent[0].subject.contains("开通交易") && sent[0].text.contains("654321"));
}

/// 插入一张私信报价卡片，`expiry` 为 Unix 秒。
async fn offer(pool: &PgPool, seller: i64, hash_byte: u8, expiry: i64) -> i64 {
    let addr = [hash_byte; 20];
    forum_db::trade::insert_offer(
        pool,
        forum_db::trade::NewOffer {
            listing_id: None,
            conversation_id: Some(1),
            seller_id: seller,
            buyer_id: None,
            replaces_id: None,
            chain_id: 421_614,
            verifying_contract: &addr,
            seller: &addr,
            buyer: &addr,
            token: &addr,
            amount: "5000000",
            fee_payer: 0,
            max_fee_bps: 100,
            max_base_gas: "100000",
            max_min_fee: "1000000",
            bond: "0",
            late_slash_bps: 0,
            instant: false,
            ship_window: 86_400,
            confirm_window: 86_400,
            listing_hash: &[0; 32],
            expiry,
            max_fills: 1,
            nonce: "1",
            signature: &[1],
            offer_hash: &[hash_byte; 32],
        },
    )
    .await
    .unwrap()
    .unwrap()
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn expired_offers_are_marked(pool: PgPool) {
    let (user, _) = forum_db::users::upsert_by_email(&pool, "s@example.com")
        .await
        .unwrap();
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let old = offer(&pool, user.id, 1, now - 10).await;
    let live = offer(&pool, user.id, 2, now + 3600).await;
    let cancelled = offer(&pool, user.id, 3, now - 10).await;
    sqlx::query("UPDATE offers SET status = 'cancelled' WHERE id = $1")
        .bind(cancelled)
        .execute(&pool)
        .await
        .unwrap();

    assert_eq!(forum_worker::expire_offers(&pool).await.unwrap(), 1);
    assert_eq!(forum_worker::expire_offers(&pool).await.unwrap(), 0);
    let status = |id: i64| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, String>("SELECT status::text FROM offers WHERE id = $1")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    assert_eq!(status(old).await, "expired");
    assert_eq!(status(live).await, "pending");
    assert_eq!(
        status(cancelled).await,
        "cancelled",
        "closed cards stay closed"
    );
}
