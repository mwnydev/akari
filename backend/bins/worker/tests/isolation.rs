//! 发件箱逐条隔离（第 9 轮评审）：一条坏消息只进它自己的死信，不挡同批与之后的消息；临时失败退避重试；
//! 认领后崩溃的消息租期过后重投；多个 worker 并行不重复投递；死信提醒去重；站长重新投递。
//! 需要 PostgreSQL（`DATABASE_URL`）。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use forum_db::{PgPool, outbox};
use forum_mail::{Email, MailError, Mailer};
use serde_json::json;

#[derive(Default)]
struct Recorder {
    sent: Mutex<Vec<Email>>,
    down: AtomicBool,
    delay: Option<Duration>,
}

impl Mailer for Recorder {
    fn name(&self) -> &'static str {
        "recorder"
    }
    async fn send(&self, email: &Email) -> Result<(), MailError> {
        if let Some(d) = self.delay {
            tokio::time::sleep(d).await;
        }
        if self.down.load(Ordering::SeqCst) {
            return Err(MailError::Transport("503 service unavailable".into()));
        }
        self.sent.lock().unwrap().push(email.clone());
        Ok(())
    }
}

fn email(to: &str) -> serde_json::Value {
    json!({"to": to, "at": "2026-10-19T00:00:00Z"})
}

fn reply(to: i64, post_id: i64) -> serde_json::Value {
    json!({"to": to, "from": 2, "topic_id": 3, "post_id": post_id, "floor": 2})
}

async fn drain(pool: &PgPool, mailer: &Recorder) -> usize {
    forum_worker::drain_once(pool, mailer, None, None)
        .await
        .unwrap()
}

/// 一直投递到没有到期的消息。
async fn drain_all(pool: &PgPool, mailer: &Recorder) {
    while drain(pool, mailer).await > 0 {}
}

/// `(attempts, 已发送, 死信, last_error)`
async fn state(pool: &PgPool, id: i64) -> (i32, bool, bool, Option<String>) {
    sqlx::query_as(
        "SELECT attempts, sent_at IS NOT NULL, failed_at IS NOT NULL, last_error FROM outbox WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn notified_posts(pool: &PgPool, user: i64) -> Vec<i64> {
    sqlx::query_scalar(
        "SELECT (payload->>'post_id')::bigint FROM notifications WHERE user_id = $1 ORDER BY id",
    )
    .bind(user)
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn alerts(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT dedupe_key FROM outbox WHERE kind = 'staff_alert' AND dedupe_key LIKE 'outbox.%' ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn make_due(pool: &PgPool) {
    sqlx::query("UPDATE outbox SET next_attempt_at = now() WHERE sent_at IS NULL")
        .execute(pool)
        .await
        .unwrap();
}

/// 第 9 轮的事故：一条违反约束的通知。它进死信，排在它后面的同类与别的类型照常送达；修好约束后重新投递成功。
#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn poison_message_is_dead_lettered_and_others_still_deliver(pool: PgPool) {
    // 模拟约束与代码不一致：带 poison 字段的通知写不进去
    sqlx::query("ALTER TABLE notifications ADD CONSTRAINT poison CHECK (NOT payload ? 'poison')")
        .execute(&pool)
        .await
        .unwrap();
    let mut poison_payload = reply(1, 100);
    poison_payload["poison"] = json!(true);
    let poison = outbox::enqueue(&pool, "notify.reply", &poison_payload, 0)
        .await
        .unwrap();
    let same_kind = outbox::enqueue(&pool, "notify.reply", &reply(1, 101), 0)
        .await
        .unwrap();
    let other_kind = outbox::enqueue(&pool, "notify.mention", &reply(1, 102), 0)
        .await
        .unwrap();
    let mail = outbox::enqueue(&pool, "email.new_device", &email("a@example.com"), 0)
        .await
        .unwrap();
    let mailer = Recorder::default();
    assert_eq!(drain(&pool, &mailer).await, 4);

    let (attempts, sent, dead, error) = state(&pool, poison).await;
    assert_eq!((attempts, sent, dead), (1, false, true));
    let error = error.unwrap();
    assert!(error.contains("violates check constraint"), "{error}");
    for id in [same_kind, other_kind, mail] {
        assert_eq!(state(&pool, id).await, (1, true, false, None));
    }
    assert_eq!(notified_posts(&pool, 1).await, vec![101, 102]);
    assert_eq!(mailer.sent.lock().unwrap().len(), 1);
    // 之后进来的消息照常投递
    outbox::enqueue(&pool, "notify.reply", &reply(1, 103), 0)
        .await
        .unwrap();
    drain_all(&pool, &mailer).await;
    assert_eq!(notified_posts(&pool, 1).await, vec![101, 102, 103]);
    assert_eq!(
        alerts(&pool).await,
        vec![forum_worker::dead_letter_event("notify.reply")]
    );

    // 修好之后重新投递：payload 原样保留，送达
    sqlx::query("ALTER TABLE notifications DROP CONSTRAINT poison")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(outbox::requeue(&pool, &[poison]).await.unwrap().len(), 1);
    drain_all(&pool, &mailer).await;
    assert!(state(&pool, poison).await.1);
    assert_eq!(notified_posts(&pool, 1).await, vec![101, 102, 103, 100]);
}

/// 服务商临时故障：按退避重试（期间不再取出），恢复后送达，幂等键不变。
#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn transient_failure_backs_off_then_succeeds(pool: PgPool) {
    let id = outbox::enqueue(&pool, "email.new_device", &email("a@example.com"), 0)
        .await
        .unwrap();
    let mailer = Recorder::default();
    mailer.down.store(true, Ordering::SeqCst);
    for attempt in 1..=3 {
        assert_eq!(drain(&pool, &mailer).await, 1);
        let (attempts, sent, dead, error) = state(&pool, id).await;
        assert_eq!((attempts, sent, dead), (attempt, false, false));
        assert_eq!(error.as_deref(), Some("transport: 503 service unavailable"));
        let wait: f64 = sqlx::query_scalar(
            "SELECT extract(epoch FROM next_attempt_at - now())::float8 FROM outbox WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let base = f64::from(1 << attempt);
        assert!(wait > base * 0.5 - 1.0 && wait <= base, "{wait}");
        assert_eq!(drain(&pool, &mailer).await, 0, "backing off");
        make_due(&pool).await;
    }
    mailer.down.store(false, Ordering::SeqCst);
    assert_eq!(drain(&pool, &mailer).await, 1);
    let (attempts, sent, dead, _) = state(&pool, id).await;
    assert_eq!((attempts, sent, dead), (4, true, false));
    let sent = mailer.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].idempotency_key, format!("outbox-{id}"));
}

/// 认领后崩溃（没有标记）：租期过后重新投递；原认领者迟到的标记不生效，站内写入也不会重复。
/// 反复崩溃到次数用尽的消息进死信。
#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn crashed_claims_are_retried_after_the_lease(pool: PgPool) {
    let id = outbox::enqueue(&pool, "notify.reply", &reply(1, 100), 0)
        .await
        .unwrap();
    // 「崩溃的 worker」认领了它（租期 1 秒），然后就没有下文
    let crashed = outbox::claim(&pool, 10, 1.0).await.unwrap();
    assert_eq!(crashed[0].attempts, 1);
    let mailer = Recorder::default();
    assert_eq!(drain(&pool, &mailer).await, 0, "claimed by someone else");
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(drain(&pool, &mailer).await, 1);
    assert_eq!(state(&pool, id).await, (2, true, false, None));
    assert!(
        !outbox::mark_sent(&pool, id, crashed[0].attempts)
            .await
            .unwrap(),
        "stale claim cannot mark"
    );
    assert_eq!(
        outbox::mark_failed(&pool, id, crashed[0].attempts, "late", false)
            .await
            .unwrap(),
        None
    );
    assert_eq!(notified_posts(&pool, 1).await, vec![100]);

    // 次数用尽后又被认领过一次（上次投递中崩溃）：不再投递，进死信
    let stuck = outbox::enqueue(&pool, "notify.reply", &reply(1, 101), 0)
        .await
        .unwrap();
    sqlx::query("UPDATE outbox SET attempts = $2 WHERE id = $1")
        .bind(stuck)
        .bind(outbox::MAX_ATTEMPTS)
        .execute(&pool)
        .await
        .unwrap();
    drain(&pool, &mailer).await;
    let (_, sent, dead, error) = state(&pool, stuck).await;
    assert!(!sent && dead);
    assert!(error.unwrap().contains("abandoned"));
    assert_eq!(notified_posts(&pool, 1).await, vec![100]);
}

/// 两个 worker 并行投递：每条消息恰好投递一次。
#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn concurrent_drainers_do_not_double_deliver(pool: PgPool) {
    const N: i64 = 40;
    for i in 0..N {
        outbox::enqueue(
            &pool,
            "email.new_device",
            &email(&format!("u{i}@example.com")),
            0,
        )
        .await
        .unwrap();
        outbox::enqueue(&pool, "notify.reply", &reply(1, i), 0)
            .await
            .unwrap();
    }
    let mailer = Recorder {
        delay: Some(Duration::from_millis(5)),
        ..Default::default()
    };
    let (a, b) = tokio::join!(
        async {
            let mut n = 0;
            loop {
                let got = drain(&pool, &mailer).await;
                if got == 0 {
                    return n;
                }
                n += got;
            }
        },
        async {
            let mut n = 0;
            loop {
                let got = drain(&pool, &mailer).await;
                if got == 0 {
                    return n;
                }
                n += got;
            }
        }
    );
    assert_eq!(a + b, 2 * N as usize);
    assert!(a > 0 && b > 0, "both drainers worked: {a} / {b}");
    let mut per_key: BTreeMap<String, usize> = BTreeMap::new();
    for e in mailer.sent.lock().unwrap().iter() {
        *per_key.entry(e.idempotency_key.clone()).or_default() += 1;
    }
    assert_eq!(per_key.len(), N as usize);
    assert!(per_key.values().all(|&n| n == 1));
    let mut posts = notified_posts(&pool, 1).await;
    posts.sort_unstable();
    assert_eq!(posts, (0..N).collect::<Vec<_>>());
    let unsent: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox WHERE sent_at IS NULL")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(unsent, 0);
}

/// 死信提醒：同一种消息一小时内只提醒一次，不同类型各自提醒；告警消息自己进死信时不再告警。
#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn dead_letter_alerts_are_deduped_per_kind(pool: PgPool) {
    for kind in ["no.such.a", "no.such.a", "no.such.b"] {
        outbox::enqueue(&pool, kind, &json!({}), 0).await.unwrap();
    }
    // payload 不对的告警
    let bad_alert = outbox::enqueue(&pool, outbox::KIND_STAFF_ALERT, &json!({}), 0)
        .await
        .unwrap();
    let mailer = Recorder::default();
    drain(&pool, &mailer).await;
    assert!(state(&pool, bad_alert).await.2);
    assert_eq!(
        alerts(&pool).await,
        vec![
            forum_worker::dead_letter_event("no.such.a"),
            forum_worker::dead_letter_event("no.such.b"),
        ]
    );
    let text: String = sqlx::query_scalar(
        "SELECT payload->>'text' FROM outbox WHERE dedupe_key = 'outbox.dead_letter.no.such.b'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        text.contains("no.such.b") && text.contains("unknown outbox kind"),
        "{text}"
    );
    // 一小时内再来一条同类死信：不再提醒
    outbox::enqueue(&pool, "no.such.a", &json!({}), 0)
        .await
        .unwrap();
    drain_all(&pool, &mailer).await;
    assert_eq!(alerts(&pool).await.len(), 2);
}

/// 合并点赞通知进了死信：这一楼的待发标记被清掉，之后的新赞照常排下一条通知。
#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn dead_like_digest_does_not_silence_the_post(pool: PgPool) {
    use forum_db::floor;
    let post: i64 = sqlx::query_scalar(
        "INSERT INTO posts (topic_id, floor, author_id, html, plain_text) VALUES (7, 3, 1, '<p>x</p>', 'x') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("ALTER TABLE notifications ADD CONSTRAINT poison CHECK (kind <> 'like')")
        .execute(&pool)
        .await
        .unwrap();
    floor::like(&pool, post, 2, true, 0.0).await.unwrap();
    let mailer = Recorder::default();
    drain_all(&pool, &mailer).await;
    let dead: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox WHERE kind = 'notify.likes' AND failed_at IS NOT NULL",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(dead, 1);

    sqlx::query("ALTER TABLE notifications DROP CONSTRAINT poison")
        .execute(&pool)
        .await
        .unwrap();
    floor::like(&pool, post, 3, true, 0.0).await.unwrap();
    drain_all(&pool, &mailer).await;
    let likes: Vec<serde_json::Value> =
        sqlx::query_scalar("SELECT payload FROM notifications WHERE user_id = 1 AND kind = 'like'")
            .fetch_all(&pool)
            .await
            .unwrap();
    // 死信那条的赞也算在里面
    assert_eq!(likes.len(), 1);
    assert_eq!(likes[0]["count"], 2);
}

/// 巡检：按类型统计、过期死信丢弃。
#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn report_counts_and_expires_dead_letters(pool: PgPool) {
    outbox::enqueue(&pool, "no.such.kind", &json!({}), 0)
        .await
        .unwrap();
    let mailer = Recorder::default();
    drain(&pool, &mailer).await;
    assert_eq!(forum_worker::outbox_report(&pool).await.unwrap(), 0);
    sqlx::query(
        "UPDATE outbox SET failed_at = now() - interval '15 days' WHERE failed_at IS NOT NULL",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(forum_worker::outbox_report(&pool).await.unwrap(), 1);
    assert!(
        outbox::dead_letters(&pool, None, None, 10)
            .await
            .unwrap()
            .is_empty()
    );
}

/// 代码里每一种站内通知类型都要能写进 `notifications`（第 9 轮：新增的 `auction` 没加进 CHECK 约束）。
/// 类型从 `forum_db::notifications` 的 `KIND_*` 常量里读出（worker 写通知只用这些常量）。
#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn every_notification_kind_is_allowed(pool: PgPool) {
    let src = include_str!("../../../crates/db/src/notifications.rs");
    let kinds: Vec<&str> = src
        .lines()
        .filter_map(|l| l.trim().strip_prefix("pub const KIND_"))
        .map(|rest| rest.split('"').nth(1).unwrap())
        .collect();
    assert!(kinds.len() >= 9, "{kinds:?}");
    let items: Vec<forum_db::notifications::NewNotification> = kinds
        .iter()
        .map(|k| forum_db::notifications::NewNotification {
            user_id: 1,
            kind: k,
            actor_id: None,
            payload: json!({}),
        })
        .collect();
    let inserted = forum_db::notifications::insert(&pool, &items)
        .await
        .unwrap();
    assert_eq!(inserted.len(), kinds.len());
}

/// 拍卖付款邮件（第 10 轮合入的新类型）走同一套失败分类：通道故障按退避重试后送达，payload 不对直接进死信。
#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn auction_payment_emails_retry_or_dead_letter(pool: PgPool) {
    let user: i64 =
        sqlx::query_scalar("INSERT INTO users (email) VALUES ('w@x.test') RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap();
    let deadline = time::OffsetDateTime::from_unix_timestamp(1_792_000_000).unwrap();
    forum_db::auctions::payment_email(&pool, user, 7, deadline, true)
        .await
        .unwrap();
    let good: i64 = sqlx::query_scalar("SELECT id FROM outbox WHERE kind = $1")
        .bind(forum_db::auctions::EMAIL_PAYMENT)
        .fetch_one(&pool)
        .await
        .unwrap();
    let bad = outbox::enqueue(
        &pool,
        forum_db::auctions::EMAIL_PAYMENT,
        &json!({"to": "w@x.test", "auction_id": 7}),
        0,
    )
    .await
    .unwrap();

    let mailer = Recorder::default();
    mailer.down.store(true, Ordering::SeqCst);
    drain(&pool, &mailer).await;
    let (attempts, sent, dead, _) = state(&pool, good).await;
    assert_eq!(
        (attempts, sent, dead),
        (1, false, false),
        "transient: retried"
    );
    let (attempts, sent, dead, _) = state(&pool, bad).await;
    assert_eq!(
        (attempts, sent, dead),
        (1, false, true),
        "bad payload: dead letter"
    );

    mailer.down.store(false, Ordering::SeqCst);
    make_due(&pool).await;
    drain_all(&pool, &mailer).await;
    assert!(state(&pool, good).await.1);
    assert_eq!(
        alerts(&pool).await,
        [forum_worker::dead_letter_event(
            forum_db::auctions::EMAIL_PAYMENT
        )]
    );
    let sent = mailer.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].to, "w@x.test");
    assert!(sent[0].text.contains("2026-10-14 17:46 UTC"));
}
