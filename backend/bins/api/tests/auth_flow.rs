//! 登录全流程：需要 PostgreSQL（`DATABASE_URL`）与 Valkey（`VALKEY_URL`，未设置则跳过）。
//! 验证码直接从发件箱读取（测试钩子），不依赖邮件服务。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{build, call, clear_resend, jar, latest_code, login, unique_email};
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

/// 一条 Set-Cookie 的名字与小写的属性列表。
fn parse_set_cookie(c: &str) -> (&str, Vec<String>) {
    let mut parts = c.split(';');
    let name = parts.next().unwrap().split('=').next().unwrap().trim();
    (name, parts.map(|a| a.trim().to_ascii_lowercase()).collect())
}

/// ADR 0012：论坛主站在可注册域名 `akari.cc` 本身上，Cookie 一旦带 `Domain` 就会发给钱包、后台等所有子域名。
/// 论坛发出的每一条 Set-Cookie 都必须 host-only（没有 Domain 属性）、Path=/，带值的还必须 HttpOnly、
/// Secure（测试状态与生产一样 `cookie_secure = true`）、SameSite=Lax。删除用的空 Cookie（`Max-Age=0`）
/// 不带值，只要求域与路径和原 Cookie 一致。改动 Cookie 属性的 PR 需两人审查。
fn assert_cookie_guard(set_cookies: &[String]) {
    for c in set_cookies {
        let (_, attrs) = parse_set_cookie(c);
        assert!(!attrs.iter().any(|a| a.starts_with("domain")), "{c}");
        assert!(attrs.iter().any(|a| a == "path=/"), "{c} missing path=/");
        if attrs.iter().any(|a| a == "max-age=0") {
            continue;
        }
        for want in ["httponly", "secure", "samesite=lax"] {
            assert!(attrs.iter().any(|a| a == want), "{c} missing {want}");
        }
    }
}

fn cookie_value<'a>(set_cookies: &'a [String], name: &str) -> Option<&'a str> {
    set_cookies
        .iter()
        .find_map(|c| c.split(';').next()?.strip_prefix(name)?.strip_prefix('='))
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn session_cookies_are_host_only_http_only_secure_lax(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let email = unique_email();

    // 登录：会话与设备 Cookie
    let r = login(&app, &db, &email, &[]).await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    let names: Vec<_> = r.cookies.iter().map(|c| parse_set_cookie(c).0).collect();
    assert!(
        names.contains(&"sid") && names.contains(&"did"),
        "{names:?}"
    );
    assert_cookie_guard(&r.cookies);
    let cookies = jar(&r.cookies);
    let sid = cookie_value(&r.cookies, "sid").unwrap().to_owned();

    // 普通已登录请求不重发会话 Cookie；若重发（例如以后开启续期），属性与 ID 都不能变
    let r = call(&app, "GET", "/api/v1/me", None, &cookies).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_cookie_guard(&r.cookies);
    assert!(cookie_value(&r.cookies, "sid").is_none_or(|v| v == sid));

    // 改用户名不轮换、不清除会话
    let r = call(
        &app,
        "PUT",
        "/api/v1/me/username",
        Some(json!({"username": "cookie_guard"})),
        &cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert_cookie_guard(&r.cookies);
    assert!(cookie_value(&r.cookies, "sid").is_none_or(|v| v == sid));
    let r = call(&app, "GET", "/api/v1/me", None, &cookies).await;
    assert_eq!(r.body["username"], "cookie_guard");

    // 再次登录（同一设备）轮换会话 ID
    clear_resend(&email).await;
    let r = login(&app, &db, &email, &cookies).await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert_cookie_guard(&r.cookies);
    let rotated = cookie_value(&r.cookies, "sid").unwrap();
    assert_ne!(rotated, sid);
    let cookies = jar(&r.cookies);

    // 退出：删除会话 Cookie 的那一条同样 host-only，否则浏览器删不掉原来的 Cookie
    let r = call(&app, "POST", "/api/v1/auth/logout", None, &cookies).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert_eq!(cookie_value(&r.cookies, "sid"), Some(""), "{:?}", r.cookies);
    assert_cookie_guard(&r.cookies);
}

/// 改 Valkey 里的会话记录（模拟时间流逝）；`ttl` 是写回后 Valkey 里的剩余时间。
async fn age_session(
    cookies: &[String],
    ttl: std::time::Duration,
    f: impl FnOnce(&mut tower_sessions::session::Record),
) {
    let kv = forum_kv::pool(&std::env::var("VALKEY_URL").unwrap()).unwrap();
    let key = common::session_key(cookies);
    let raw = forum_kv::ephemeral::get(&kv, &key).await.unwrap().unwrap();
    let mut record: tower_sessions::session::Record = serde_json::from_str(&raw).unwrap();
    f(&mut record);
    forum_kv::ephemeral::put(&kv, &key, &serde_json::to_string(&record).unwrap(), ttl)
        .await
        .unwrap();
}

async fn session_record(cookies: &[String]) -> Option<tower_sessions::session::Record> {
    let kv = forum_kv::pool(&std::env::var("VALKEY_URL").unwrap()).unwrap();
    forum_kv::ephemeral::get(&kv, &common::session_key(cookies))
        .await
        .unwrap()
        .map(|raw| serde_json::from_str(&raw).unwrap())
}

const HOUR: std::time::Duration = std::time::Duration::from_secs(3600);

fn ago(d: time::Duration) -> serde_json::Value {
    json!((time::OffsetDateTime::now_utc() - d).unix_timestamp())
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn active_sessions_extend_hourly_up_to_90_days(db: PgPool) {
    use forum_api::routes::{SESSION_LOGIN_AT, SESSION_TOUCHED_AT};
    use time::Duration;

    let Some(app) = build(db.clone()) else { return };
    let r = login(&app, &db, &unique_email(), &[]).await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    let cookies = jar(&r.cookies);
    let soon = time::OffsetDateTime::now_utc() + Duration::days(1);

    // 一小时内碰过：不写会话，过期时间不动，也不重发 Cookie
    age_session(&cookies, HOUR, |s| {
        s.expiry_date = soon;
        s.data
            .insert(SESSION_TOUCHED_AT.into(), ago(Duration::minutes(59)));
    })
    .await;
    let r = call(&app, "GET", "/api/v1/me", None, &cookies).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(
        !r.cookies.iter().any(|c| c.starts_with("sid=")),
        "{:?}",
        r.cookies
    );
    let s = session_record(&cookies).await.unwrap();
    assert_eq!(s.expiry_date.unix_timestamp(), soon.unix_timestamp());

    // 超过一小时：顺延到 30 天后，Cookie 也一起续期
    age_session(&cookies, HOUR, |s| {
        s.data
            .insert(SESSION_TOUCHED_AT.into(), ago(Duration::minutes(61)));
    })
    .await;
    let r = call(&app, "GET", "/api/v1/me", None, &cookies).await;
    assert_eq!(r.status, StatusCode::OK);
    let set = r.cookies.iter().find(|c| c.starts_with("sid=")).unwrap();
    assert!(set.contains("Max-Age=2592000"), "{set}");
    let s = session_record(&cookies).await.unwrap();
    let left = s.expiry_date - time::OffsetDateTime::now_utc();
    assert!(
        left > Duration::days(29) && left <= Duration::days(30),
        "{left}"
    );

    // 登录满 90 天：不管多活跃，都要重新登录
    age_session(&cookies, HOUR, |s| {
        s.data
            .insert(SESSION_LOGIN_AT.into(), ago(Duration::days(90)));
    })
    .await;
    let r = call(&app, "GET", "/api/v1/me", None, &cookies).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert!(session_record(&cookies).await.is_none());
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn idle_sessions_expire_after_30_days(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let r = login(&app, &db, &unique_email(), &[]).await;
    let cookies = jar(&r.cookies);
    let set = r.cookies.iter().find(|c| c.starts_with("sid=")).unwrap();
    assert!(set.contains("Max-Age=2592000"), "{set}");
    // 会话记录在 Valkey 里按 expiry_date（登录或上次顺延后 30 天）过期，过期后不再算登录
    let s = session_record(&cookies).await.unwrap();
    let left = s.expiry_date - time::OffsetDateTime::now_utc();
    assert!(left > time::Duration::days(29), "{left}");
    age_session(&cookies, std::time::Duration::from_millis(50), |_| ()).await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let r = call(&app, "GET", "/api/v1/me", None, &cookies).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
}
