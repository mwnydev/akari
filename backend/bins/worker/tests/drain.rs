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
