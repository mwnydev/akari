//! 登录全流程：需要 PostgreSQL（`DATABASE_URL`）与 Valkey（`VALKEY_URL`，未设置则跳过）。
//! 验证码直接从发件箱读取（测试钩子），不依赖邮件服务。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{SECRET, build, call, jar, latest_code, login, unique_email};
use forum_auth::CodeStore;
use forum_db::PgPool;
use serde_json::json;
use tower::ServiceExt;

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
