//! API 集成测试共用：组装应用、发请求、登录。
//! 需要 PostgreSQL（`DATABASE_URL`，sqlx::test 每个用例建临时库）与 Valkey（`VALKEY_URL`，未设置则跳过）。
//! 每个用例的 Valkey 键带随机前缀，临时库之间重复的 ID 不会互相影响。

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)] // 测试代码；各测试文件只用其中一部分

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{Request, StatusCode, header};
use axum_extra::extract::cookie::Key;
use forum_api::{AppState, app};
use forum_auth::{AnyCaptcha, CodeStore, NoopCaptcha};
use forum_db::PgPool;
use serde_json::{Value, json};
use tower::ServiceExt;

pub const ORIGIN: &str = "https://forum.test";
pub const SECRET: [u8; 64] = [9; 64];

fn nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

pub fn build(db: PgPool) -> Option<Router> {
    let Ok(url) = std::env::var("VALKEY_URL") else {
        eprintln!("VALKEY_URL not set, skipping valkey-backed test");
        return None;
    };
    let kv = forum_kv::pool(&url).unwrap();
    let state = AppState {
        db,
        kv: kv.clone(),
        codes: CodeStore::new(kv.clone(), &SECRET).unwrap(),
        captcha: AnyCaptcha::Noop(NoopCaptcha),
        device_key: Key::from(&SECRET),
        allowed_origins: Arc::from(vec![ORIGIN.to_owned()]),
        cookie_secure: true,
        cache: forum_kv::ForumCache::new(kv, format!("test{}:", nanos())),
        internal_hosts: Arc::from(vec!["forum.test".to_owned()]),
    };
    // 每个用例用不同的假 IP，避免 IP 限流互相影响
    Some(app(state, &SECRET).layer(MockConnectInfo(SocketAddr::from((rand_ip(), 40000)))))
}

pub fn rand_ip() -> [u8; 4] {
    let n = nanos() as u32;
    [10, (n >> 16) as u8, (n >> 8) as u8, n as u8]
}

pub fn unique_email() -> String {
    format!("User{}@Example.com", nanos())
}

pub struct Res {
    pub status: StatusCode,
    pub cookies: Vec<String>,
    pub body: Value,
}

pub async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    cookies: &[String],
) -> Res {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::ORIGIN, ORIGIN);
    if !cookies.is_empty() {
        req = req.header(header::COOKIE, cookies.join("; "));
    }
    let req = match body {
        Some(b) => req
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(b.to_string())),
        None => req.body(Body::empty()),
    }
    .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let cookies = res
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_owned())
        .collect();
    let bytes = to_bytes(res.into_body(), 1 << 22).await.unwrap();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    Res {
        status,
        cookies,
        body,
    }
}

/// 把 Set-Cookie 转成请求用的 `name=value`。
pub fn jar(set_cookies: &[String]) -> Vec<String> {
    set_cookies
        .iter()
        .map(|c| c.split(';').next().unwrap().to_owned())
        .collect()
}

pub async fn latest_code(db: &PgPool) -> String {
    sqlx::query_scalar("SELECT payload->>'code' FROM outbox WHERE kind = 'email.login_code' ORDER BY id DESC LIMIT 1")
        .fetch_one(db)
        .await
        .unwrap()
}

pub async fn login(app: &Router, db: &PgPool, email: &str, cookies: &[String]) -> Res {
    let r = call(
        app,
        "POST",
        "/api/v1/auth/email/start",
        Some(json!({"email": email})),
        cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{:?}", r.body);
    let code = latest_code(db).await;
    call(
        app,
        "POST",
        "/api/v1/auth/email/verify",
        Some(json!({"email": email, "code": code})),
        cookies,
    )
    .await
}

/// 登录一个新用户，返回 `(用户 ID, 会话 Cookie)`。
pub async fn user(app: &Router, db: &PgPool) -> (i64, Vec<String>) {
    let r = login(app, db, &unique_email(), &[]).await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    (r.body["user"]["id"].as_i64().unwrap(), jar(&r.cookies))
}

/// 把用户注册时间改到 2 天前，解除新用户限制。
pub async fn age(db: &PgPool, user_id: i64) {
    sqlx::query("UPDATE users SET created_at = now() - interval '2 days' WHERE id = $1")
        .bind(user_id)
        .execute(db)
        .await
        .unwrap();
}
