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
    assert_eq!(
        forum_worker::drain_once(&pool, &mailer, None, None)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        forum_worker::drain_once(&pool, &mailer, None, None)
            .await
            .unwrap(),
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
    assert_eq!(
        forum_worker::drain_once(&pool, &mailer, None, None)
            .await
            .unwrap(),
        2
    );

    let rows: Vec<(i64, i32, Option<String>)> =
        sqlx::query_as("SELECT id, attempts, last_error FROM outbox ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(rows[0], (id, 1, Some("transport: down".into())));
    assert_eq!(rows[1].0, bad);
    assert_eq!(rows[1].2.as_deref(), Some("unknown outbox kind"));
    // 退避期内不再取出
    assert_eq!(
        forum_worker::drain_once(&pool, &mailer, None, None)
            .await
            .unwrap(),
        0
    );
}

async fn notifications_of(
    pool: &PgPool,
    user: i64,
) -> Vec<(String, Option<i64>, serde_json::Value)> {
    sqlx::query_as(
        "SELECT kind, actor_id, payload FROM notifications WHERE user_id = $1 ORDER BY id",
    )
    .bind(user)
    .fetch_all(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn in_site_notifications_are_stored_not_emailed(pool: PgPool) {
    let id = outbox::enqueue(
        &pool,
        "notify.reply",
        &json!({"to": 1, "from": 2, "topic_id": 3, "post_id": 4, "floor": 2}),
        0,
    )
    .await
    .unwrap();
    outbox::enqueue(
        &pool,
        "notify.offer",
        &json!({"to": 1, "from": 5, "offer_id": 9, "conversation_id": 8, "event": "created"}),
        0,
    )
    .await
    .unwrap();
    outbox::enqueue(
        &pool,
        "notify.withdraw_address_added",
        &json!({"to": 1, "address": "0xabc"}),
        0,
    )
    .await
    .unwrap();
    let mailer = Recorder::default();
    assert_eq!(
        forum_worker::drain_once(&pool, &mailer, None, None)
            .await
            .unwrap(),
        3
    );
    assert!(mailer.sent.lock().unwrap().is_empty());
    let sent: Option<time::OffsetDateTime> =
        sqlx::query_scalar("SELECT sent_at FROM outbox WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(sent.is_some());
    assert_eq!(
        notifications_of(&pool, 1).await,
        vec![
            (
                "reply".into(),
                Some(2),
                json!({"topic_id": 3, "post_id": 4, "floor": 2})
            ),
            (
                "offer".into(),
                Some(5),
                json!({"offer_id": 9, "conversation_id": 8, "event": "created"})
            ),
            (
                "system".into(),
                None,
                json!({"address": "0xabc", "event": "withdraw_address_added"})
            ),
        ]
    );
    // 缺收件人的消息投递失败、按退避重试
    let bad = outbox::enqueue(&pool, "notify.mention", &json!({"from": 2}), 0)
        .await
        .unwrap();
    forum_worker::drain_once(&pool, &mailer, None, None)
        .await
        .unwrap();
    let attempts: i32 = sqlx::query_scalar("SELECT attempts FROM outbox WHERE id = $1")
        .bind(bad)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(attempts, 1);
}

/// 窗口内的赞合成一条 `like` 通知（楼主自己的赞不算）；投递后再来的赞排下一条，全被取消时不通知。
#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn likes_are_merged_into_one_notification(pool: PgPool) {
    use forum_db::floor;
    let post: i64 = sqlx::query_scalar(
        "INSERT INTO posts (topic_id, floor, author_id, html, plain_text) VALUES (7, 3, 1, '<p>x</p>', 'x') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let mailer = Recorder::default();
    let drain = || forum_worker::drain_once(&pool, &mailer, None, None);
    let pending = || {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM outbox WHERE kind = 'notify.likes' AND sent_at IS NULL",
        )
        .fetch_one(&pool)
    };

    // 窗口为 0：马上到期，便于测试
    floor::like(&pool, post, 2, true, 0.0).await.unwrap();
    floor::like(&pool, post, 3, true, 0.0).await.unwrap();
    floor::like(&pool, post, 1, false, 0.0).await.unwrap();
    assert_eq!(pending().await.unwrap(), 1, "one digest per window");
    assert_eq!(drain().await.unwrap(), 1);
    assert_eq!(
        notifications_of(&pool, 1).await,
        vec![(
            "like".into(),
            Some(3),
            json!({"topic_id": 7, "post_id": post, "floor": 3, "count": 2, "likers": [3, 2]})
        )]
    );

    // 投递之后的新赞排下一条，只统计新的；赞了又取消的不通知
    floor::like(&pool, post, 4, true, 0.0).await.unwrap();
    assert_eq!(pending().await.unwrap(), 1);
    floor::unlike(&pool, post, 4).await.unwrap();
    assert_eq!(drain().await.unwrap(), 1);
    assert_eq!(pending().await.unwrap(), 0);
    assert_eq!(notifications_of(&pool, 1).await.len(), 1);
    floor::like(&pool, post, 5, true, 0.0).await.unwrap();
    assert_eq!(drain().await.unwrap(), 1);
    let notes = notifications_of(&pool, 1).await;
    assert_eq!(notes.len(), 2);
    assert_eq!(
        (notes[1].2["count"].as_i64(), notes[1].1),
        (Some(1), Some(5))
    );
}

fn order_updated(cause: &str) -> serde_json::Value {
    json!({"id": 7, "chain_id": 1, "contract": "0x", "mode": "escrow", "chain_order_id": 1, "buyer_id": 10,
           "seller_id": 11, "state": "funded", "level": "latest", "block_number": 5, "cause": cause,
           "correction": cause == "reorg"})
}

/// 订单变化通知双方；确认级别升级只推送、不通知。indexer 写的私信卡片按 ID 推送。推送在提交后经 Valkey 发布。
#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn order_updates_notify_parties_and_push(pool: PgPool) {
    for cause in ["event", "confirmation"] {
        outbox::enqueue(&pool, "order.updated", &order_updated(cause), 0)
            .await
            .unwrap();
    }
    let (s, _) = forum_db::users::upsert_by_email(&pool, "s@example.com")
        .await
        .unwrap();
    let (b, _) = forum_db::users::upsert_by_email(&pool, "b@example.com")
        .await
        .unwrap();
    let (c, _) = forum_db::messages::get_or_create(&pool, s.id, b.id)
        .await
        .unwrap();
    let m = forum_db::messages::insert(
        &pool,
        forum_db::messages::NewMessage {
            conversation_id: c.id,
            sender_id: Some(s.id),
            kind: "text",
            body: "hi",
            offer_id: None,
            warn: false,
        },
    )
    .await
    .unwrap();
    outbox::enqueue(&pool, "push.message", &json!({"id": m.id}), 0)
        .await
        .unwrap();
    let rt = std::env::var("VALKEY_URL").ok().map(|url| {
        (
            url.clone(),
            forum_kv::Realtime::new(
                forum_kv::pool(&url).unwrap(),
                format!("wpush{}:", std::process::id()),
            ),
        )
    });
    let (got_tx, mut got) = tokio::sync::mpsc::unbounded_channel();
    if let Some((url, rt)) = &rt {
        let (url, channel) = (url.clone(), rt.channel());
        let (ready_tx, ready) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            forum_kv::realtime::listen(
                &url,
                &channel,
                || {
                    let _ = ready_tx.send(());
                },
                |e| {
                    let _ = got_tx.send(e);
                },
            )
            .await
        });
        ready.await.unwrap();
    }
    let mailer = Recorder::default();
    forum_worker::drain_once(&pool, &mailer, None, rt.as_ref().map(|(_, r)| r))
        .await
        .unwrap();
    for user in [10, 11] {
        let n = notifications_of(&pool, user).await;
        assert_eq!(n.len(), 1, "only the chain event notifies");
        assert_eq!(n[0].0, "order");
        assert_eq!(n[0].2["order_id"], 7);
    }
    if rt.is_some() {
        // 2 条通知 + 2 次订单推送 + 1 条私信
        let mut types = Vec::new();
        for _ in 0..5 {
            let e = tokio::time::timeout(std::time::Duration::from_secs(5), got.recv())
                .await
                .unwrap()
                .unwrap();
            let v: serde_json::Value = serde_json::from_str(&e.event).unwrap();
            if v["type"] == "message" {
                assert_eq!(v["message"]["id"], m.id);
                assert_eq!(
                    e.route,
                    forum_kv::realtime::Route::Users {
                        ids: c.members().to_vec()
                    }
                );
            }
            types.push(v["type"].as_str().unwrap().to_owned());
        }
        types.sort();
        assert_eq!(
            types,
            ["message", "notification", "notification", "order", "order"]
        );
    }
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn notification_partitions_roll_forward(pool: PgPool) {
    let parts = || async {
        sqlx::query_scalar::<_, String>(
            "SELECT c.relname::text FROM pg_inherits i JOIN pg_class c ON c.oid = i.inhrelid
             WHERE i.inhparent = 'notifications'::regclass ORDER BY 1",
        )
        .fetch_all(&pool)
        .await
        .unwrap()
    };
    // 迁移已建好本月与之后两个月
    assert_eq!(parts().await.len(), 3);
    assert_eq!(
        forum_db::notifications::maintain_partitions(&pool)
            .await
            .unwrap(),
        (vec![], vec![])
    );
    // 一年后：建新的三个月，删掉旧分区
    let (created, dropped): (Vec<String>, Vec<String>) = sqlx::query_as(
        "SELECT created, dropped FROM notifications_maintain(now() + interval '1 year')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(created.len(), 3);
    assert_eq!(dropped.len(), 3);
    assert_eq!(parts().await, created);
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
    assert_eq!(
        forum_worker::drain_once(&pool, &mailer, None, None)
            .await
            .unwrap(),
        1
    );
    let sent = mailer.sent.lock().unwrap();
    assert!(sent[0].subject.contains("开通交易") && sent[0].text.contains("654321"));
}

/// 插入一张私信报价卡片，`expiry` 为 Unix 秒。
async fn offer(pool: &PgPool, seller: i64, conversation: i64, hash_byte: u8, expiry: i64) -> i64 {
    let addr = [hash_byte; 20];
    forum_db::trade::insert_offer(
        pool,
        forum_db::trade::NewOffer {
            listing_id: None,
            conversation_id: Some(conversation),
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
    let (buyer, _) = forum_db::users::upsert_by_email(&pool, "b@example.com")
        .await
        .unwrap();
    let (c, _) = forum_db::messages::get_or_create(&pool, user.id, buyer.id)
        .await
        .unwrap();
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let old = offer(&pool, user.id, c.id, 1, now - 10).await;
    let live = offer(&pool, user.id, c.id, 2, now + 3600).await;
    let cancelled = offer(&pool, user.id, c.id, 3, now - 10).await;
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
