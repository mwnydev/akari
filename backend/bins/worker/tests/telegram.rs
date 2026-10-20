//! 管理员群 Telegram 告警：发件箱 `staff_alert` → Bot API `sendMessage`（本机假服务）。
//! 需要 PostgreSQL（`DATABASE_URL`）。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

use std::sync::{Arc, Mutex};

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::post;
use forum_db::{PgPool, outbox};
use forum_mail::{Email, MailError, Mailer};
use forum_worker::telegram::Telegram;
use serde_json::{Value, json};

const TOKEN: &str = "123456:SECRET-TOKEN";

struct NoMail;

impl Mailer for NoMail {
    fn name(&self) -> &'static str {
        "none"
    }
    async fn send(&self, _: &Email) -> Result<(), MailError> {
        Err(MailError::Transport("unexpected email".into()))
    }
}

#[derive(Clone, Default)]
struct Bot {
    seen: Arc<Mutex<Vec<(String, Value)>>>,
    /// 为真时像 Telegram 限流一样返回 429。
    throttled: bool,
}

async fn send_message(
    State(bot): State<Bot>,
    Path(token): Path<String>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    bot.seen.lock().unwrap().push((token, body));
    if bot.throttled {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(
                json!({"ok": false, "error_code": 429, "description": "Too Many Requests: retry after 3"}),
            ),
        );
    }
    (StatusCode::OK, Json(json!({"ok": true, "result": {}})))
}

/// 启动假 Bot API，返回基础地址。路径形如 `/bot<令牌>/sendMessage`。
async fn serve(bot: Bot) -> String {
    let app = axum::Router::new()
        .route("/{token}/sendMessage", post(send_message))
        .with_state(bot);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

async fn row(pool: &PgPool, id: i64) -> (bool, i32, Option<String>) {
    sqlx::query_as("SELECT sent_at IS NOT NULL, attempts, last_error FROM outbox WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn staff_alert_reaches_admin_group(pool: PgPool) {
    let bot = Bot::default();
    let base = serve(bot.clone()).await;
    let tg = Telegram::new(&base, TOKEN, "-1001234").unwrap();
    let id = outbox::staff_alert(&pool, "staff.test", "<b>不解析</b> 新设备登录")
        .await
        .unwrap();
    assert_eq!(
        forum_worker::drain_once(&pool, &NoMail, Some(&tg), None)
            .await
            .unwrap(),
        1
    );
    let seen = bot.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, format!("bot{TOKEN}"));
    assert_eq!(seen[0].1["chat_id"], "-1001234");
    assert_eq!(seen[0].1["text"], "[staff.test] <b>不解析</b> 新设备登录");
    // 纯文本：不设 parse_mode
    assert!(seen[0].1.get("parse_mode").is_none());
    assert!(row(&pool, id).await.0);
    assert!(!format!("{tg:?}").contains("SECRET"));
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn failures_retry_with_backoff_without_leaking_token(pool: PgPool) {
    let bot = Bot {
        throttled: true,
        ..Bot::default()
    };
    let base = serve(bot).await;
    let tg = Telegram::new(&base, TOKEN, "-1").unwrap();
    let id = outbox::staff_alert(&pool, "staff.test", "x").await.unwrap();
    forum_worker::drain_once(&pool, &NoMail, Some(&tg), None)
        .await
        .unwrap();
    let (sent, attempts, err) = row(&pool, id).await;
    assert!(!sent);
    assert_eq!(attempts, 1);
    let err = err.unwrap();
    assert!(
        err.contains("429") && err.contains("Too Many Requests"),
        "{err}"
    );
    assert!(!err.contains("SECRET"), "{err}");
    // 退避中：马上再跑不会重试
    assert_eq!(
        forum_worker::drain_once(&pool, &NoMail, Some(&tg), None)
            .await
            .unwrap(),
        0
    );

    // 连不上（端口已关闭）：错误信息里也没有带令牌的地址
    let closed = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap()
    };
    let tg = Telegram::new(&format!("http://{closed}"), TOKEN, "-1").unwrap();
    let id = outbox::staff_alert(&pool, "staff.test", "y").await.unwrap();
    forum_worker::drain_once(&pool, &NoMail, Some(&tg), None)
        .await
        .unwrap();
    let err = row(&pool, id).await.2.unwrap();
    assert!(
        !err.contains("SECRET") && !err.contains("bot123456"),
        "{err}"
    );
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn disabled_when_not_configured(pool: PgPool) {
    let id = outbox::staff_alert(&pool, "staff.test", "z").await.unwrap();
    forum_worker::drain_once(&pool, &NoMail, None, None)
        .await
        .unwrap();
    assert_eq!(row(&pool, id).await, (true, 1, None));
}
