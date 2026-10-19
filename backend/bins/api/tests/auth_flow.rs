//! 登录全流程：需要 PostgreSQL（`DATABASE_URL`）与 Valkey（`VALKEY_URL`，未设置则跳过）。
//! 验证码直接从发件箱读取（测试钩子），不依赖邮件服务。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

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

const ORIGIN: &str = "https://forum.test";
const SECRET: [u8; 64] = [9; 64];

fn build(db: PgPool) -> Option<Router> {
    let Ok(url) = std::env::var("VALKEY_URL") else {
        eprintln!("VALKEY_URL not set, skipping valkey-backed test");
        return None;
    };
    let kv = forum_kv::pool(&url).unwrap();
    let state = AppState {
        db,
        kv: kv.clone(),
        codes: CodeStore::new(kv, &SECRET).unwrap(),
        captcha: AnyCaptcha::Noop(NoopCaptcha),
        device_key: Key::from(&SECRET),
        allowed_origins: Arc::from(vec![ORIGIN.to_owned()]),
        cookie_secure: true,
    };
    // 每个用例用不同的假 IP，避免 IP 限流互相影响
    let ip: [u8; 4] = rand_ip();
    Some(app(state, &SECRET).layer(MockConnectInfo(SocketAddr::from((ip, 40000)))))
}

fn rand_ip() -> [u8; 4] {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    [10, (n >> 16) as u8, (n >> 8) as u8, n as u8]
}

fn unique_email() -> String {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("User{n}@Example.com")
}

struct Res {
    status: StatusCode,
    cookies: Vec<String>,
    body: Value,
}

async fn call(
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
    let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
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
fn jar(set_cookies: &[String]) -> Vec<String> {
    set_cookies
        .iter()
        .map(|c| c.split(';').next().unwrap().to_owned())
        .collect()
}

async fn latest_code(db: &PgPool) -> String {
    sqlx::query_scalar("SELECT payload->>'code' FROM outbox WHERE kind = 'email.login_code' ORDER BY id DESC LIMIT 1")
        .fetch_one(db)
        .await
        .unwrap()
}

async fn login(app: &Router, db: &PgPool, email: &str, cookies: &[String]) -> Res {
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

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn email_code_login_flow(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let email = unique_email();

    // 未登录
    let r = call(&app, "GET", "/api/v1/me", None, &[]).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_eq!(r.body["code"], "UNAUTHORIZED");

    let r = login(&app, &db, &email, &[]).await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert_eq!(r.body["created"], true);
    assert_eq!(r.body["user"]["email"], email.to_lowercase());
    let sid = r
        .cookies
        .iter()
        .find(|c| c.starts_with("sid="))
        .expect("session cookie");
    for attr in ["HttpOnly", "Secure", "SameSite=Lax"] {
        assert!(sid.contains(attr), "{sid} missing {attr}");
    }
    assert!(
        r.cookies.iter().any(|c| c.starts_with("did=")),
        "device cookie"
    );
    let cookies = jar(&r.cookies);

    let r = call(&app, "GET", "/api/v1/me", None, &cookies).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.body["email"], email.to_lowercase());

    // 篡改签名 Cookie 视为未登录
    let forged: Vec<String> = cookies
        .iter()
        .map(|c| {
            if c.starts_with("sid=") {
                format!("{c}x")
            } else {
                c.clone()
            }
        })
        .collect();
    assert_eq!(
        call(&app, "GET", "/api/v1/me", None, &forged).await.status,
        StatusCode::UNAUTHORIZED
    );

    let r = call(&app, "POST", "/api/v1/auth/logout", None, &cookies).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert_eq!(
        call(&app, "GET", "/api/v1/me", None, &cookies).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn session_id_rotates_on_login(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let email = unique_email();
    let first = jar(&login(&app, &db, &email, &[]).await.cookies);
    // 带着旧会话 Cookie 再登录（换个邮箱避开 60 秒重发间隔），会话 ID 必须变化
    let email2 = unique_email();
    let second = login(&app, &db, &email2, &first).await;
    let old_sid = first.iter().find(|c| c.starts_with("sid=")).unwrap();
    let new_sid = jar(&second.cookies)
        .into_iter()
        .find(|c| c.starts_with("sid="))
        .unwrap();
    assert_ne!(old_sid, &new_sid);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn new_device_notification_only_for_unknown_device(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let email = unique_email();
    let count = |db: PgPool| async move {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM outbox WHERE kind = 'email.new_device'")
            .fetch_one(&db)
            .await
            .unwrap()
    };

    // 注册（第一台设备）不提醒
    let r = login(&app, &db, &email, &[]).await;
    assert_eq!(r.status, StatusCode::OK);
    let device: Vec<String> = jar(&r.cookies)
        .into_iter()
        .filter(|c| c.starts_with("did="))
        .collect();
    assert_eq!(count(db.clone()).await, 0);

    // 同一设备再登录不提醒；清掉重发间隔键以便同邮箱再次获取验证码
    clear_resend(&email).await;
    assert_eq!(
        login(&app, &db, &email, &device).await.status,
        StatusCode::OK
    );
    assert_eq!(count(db.clone()).await, 0);

    // 没有设备 Cookie = 新设备
    clear_resend(&email).await;
    assert_eq!(login(&app, &db, &email, &[]).await.status, StatusCode::OK);
    assert_eq!(count(db.clone()).await, 1);
}

async fn clear_resend(email: &str) {
    let url = std::env::var("VALKEY_URL").unwrap();
    let kv = forum_kv::pool(&url).unwrap();
    let tag = CodeStore::new(kv.clone(), &SECRET)
        .unwrap()
        .email_tag(&email.to_lowercase());
    let mut conn = kv.get().await.unwrap();
    let _: () = redis::cmd("DEL")
        .arg(format!("auth:resend:{tag}"))
        .query_async(&mut conn)
        .await
        .unwrap();
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn wrong_code_and_errors(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let email = unique_email();
    let r = call(
        &app,
        "POST",
        "/api/v1/auth/email/start",
        Some(json!({"email": "not-an-email"})),
        &[],
    )
    .await;
    assert_eq!(
        (r.status, r.body["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("INVALID_EMAIL"))
    );

    let r = call(
        &app,
        "POST",
        "/api/v1/auth/email/start",
        Some(json!({"email": email})),
        &[],
    )
    .await;
    assert_eq!(r.status, StatusCode::ACCEPTED);
    let r = call(
        &app,
        "POST",
        "/api/v1/auth/email/start",
        Some(json!({"email": email})),
        &[],
    )
    .await;
    assert_eq!(
        (r.status, r.body["code"].as_str()),
        (StatusCode::TOO_MANY_REQUESTS, Some("RESEND_TOO_SOON"))
    );

    let code = latest_code(&db).await;
    let wrong = if code == "000000" { "111111" } else { "000000" };
    let r = call(
        &app,
        "POST",
        "/api/v1/auth/email/verify",
        Some(json!({"email": email, "code": wrong})),
        &[],
    )
    .await;
    assert_eq!(
        (r.status, r.body["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("INVALID_CODE"))
    );
    assert_eq!(r.body["message"], "验证码错误或已过期");
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn writes_require_allowed_origin(db: PgPool) {
    let Some(app) = build(db) else { return };
    for origin in [None, Some("https://evil.test")] {
        let mut req = Request::post("/api/v1/auth/logout");
        if let Some(o) = origin {
            req = req.header(header::ORIGIN, o);
        }
        let res = app
            .clone()
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "{origin:?}");
    }
    // GET 不校验 Origin
    let res = app
        .clone()
        .oneshot(Request::get("/api/v1/healthz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn openapi_lists_auth_endpoints(db: PgPool) {
    let Some(app) = build(db) else { return };
    let r = call(&app, "GET", "/api/v1/openapi.json", None, &[]).await;
    assert_eq!(r.status, StatusCode::OK);
    for path in [
        "/api/v1/auth/email/start",
        "/api/v1/auth/email/verify",
        "/api/v1/auth/logout",
        "/api/v1/me",
    ] {
        assert!(r.body["paths"].get(path).is_some(), "missing {path}");
    }
    assert!(r.body["components"]["schemas"].get("ErrorBody").is_some());
}
