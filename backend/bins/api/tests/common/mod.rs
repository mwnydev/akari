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
use forum_api::wallet::{AddressVerifier, WalletState};
use forum_api::{AppState, app};
use forum_auth::passkey::webauthn::{CreationChallengeResponse, RequestChallengeResponse, Url};
use forum_auth::passkey::{UserHandles, wallet_webauthn};
use forum_auth::{AnyCaptcha, CodeStore, NoopCaptcha};
use forum_chain::config::WalletInitVersion;
use forum_chain::kernel::PasskeyPublicKey;
use forum_core::trade::TradeLimits;
use forum_db::PgPool;
use serde_json::{Value, json};
use tower::ServiceExt;
use webauthn_authenticator_rs::WebauthnAuthenticator;
use webauthn_authenticator_rs::softpasskey::SoftPasskey;

pub const ORIGIN: &str = "https://forum.test";
pub const SECRET: [u8; 64] = [9; 64];
pub const WALLET_ORIGIN: &str = "https://wallet.test";
pub const CHAIN_ID: u64 = 421_614;
pub const ESCROW: &str = "0x4444444444444444444444444444444444444444";
pub const USDT: &str = "0x3333333333333333333333333333333333333333";

/// 假的钱包地址校验：除了 [`MISMATCHED_ADDRESS`] 一律当作与 Passkey 相符（真实校验见 wallet_flow 的 anvil 用例）。
pub struct FakeVerifier;

pub const MISMATCHED_ADDRESS: &str = "0x9999999999999999999999999999999999999999";

#[async_trait::async_trait]
impl AddressVerifier for FakeVerifier {
    async fn verify(
        &self,
        _: &WalletInitVersion,
        address: &forum_core::eip712::Address,
        _: &PasskeyPublicKey,
    ) -> anyhow::Result<bool> {
        Ok(address.to_string() != MISMATCHED_ADDRESS)
    }
}

pub fn wallet_state() -> WalletState {
    let origin = Url::parse(WALLET_ORIGIN).unwrap();
    WalletState {
        webauthn: wallet_webauthn("wallet.test", "Akari", &[origin]).unwrap(),
        origins: vec![WALLET_ORIGIN.to_owned()],
        brand: "Akari".to_owned(),
        handles: UserHandles::new(&SECRET).unwrap(),
        chain: forum_api::config::ChainConfig {
            chain_id: CHAIN_ID,
            escrow: Some(ESCROW.parse().unwrap()),
            usdt: Some(USDT.parse().unwrap()),
            wallet_init_versions: vec![forum_chain::anvil::dev_wallet_version()],
            contracts: vec![ESCROW.parse().unwrap(), USDT.parse().unwrap()],
        },
        limits: TradeLimits::default(),
        verifier: Arc::new(FakeVerifier),
        sponsor: None,
        rpc: None,
    }
}

fn nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

pub fn build(db: PgPool) -> Option<Router> {
    build_with(db, wallet_state())
}

/// 同 [`build`]，但指定钱包状态（例如换成真实的链上地址校验，或接上假的 forum-sponsor）。
pub fn build_with(db: PgPool, wallet: WalletState) -> Option<Router> {
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
        wallet: Arc::new(wallet),
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
    call_from(app, ORIGIN, method, uri, body, cookies).await
}

/// 同 [`call`]，但指定 `Origin`（钱包接口只接受钱包页面来源）。
pub async fn call_from(
    app: &Router,
    origin: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
    cookies: &[String],
) -> Res {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::ORIGIN, origin);
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
        // axum 的请求体解析错误是纯文本
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
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

/// 从钱包页面发请求。
pub async fn wallet_call(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    cookies: &[String],
) -> Res {
    call_from(app, WALLET_ORIGIN, method, uri, body, cookies).await
}

pub async fn set_username(db: &PgPool, user_id: i64, name: &str) {
    sqlx::query("UPDATE users SET username = $2 WHERE id = $1")
        .bind(user_id)
        .bind(name)
        .execute(db)
        .await
        .unwrap();
}

/// 软件 Passkey（webauthn-authenticator-rs），代替浏览器 + 平台认证器。
pub type SoftAuth = WebauthnAuthenticator<SoftPasskey>;

pub fn soft_passkey() -> SoftAuth {
    // true：声称做过用户验证（UV），钱包 Passkey 要求 UV。
    WebauthnAuthenticator::new(SoftPasskey::new(true))
}

/// 开通交易用的邮箱再验证：发码 → 从发件箱取码 → 校验。
pub async fn reverify(app: &Router, db: &PgPool, cookies: &[String]) {
    let r = wallet_call(app, "POST", "/api/v1/wallet/reverify/start", None, cookies).await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{:?}", r.body);
    let code: String = sqlx::query_scalar(
        "SELECT payload->>'code' FROM outbox WHERE kind = 'email.wallet_code' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(db)
    .await
    .unwrap();
    let r = wallet_call(
        app,
        "POST",
        "/api/v1/wallet/reverify/verify",
        Some(json!({"code": code})),
        cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{:?}", r.body);
}

/// 用软件 Passkey 完成注册，返回 finish 的响应。
pub async fn register_passkey(app: &Router, auth: &mut SoftAuth, cookies: &[String]) -> Res {
    let r = wallet_call(
        app,
        "POST",
        "/api/v1/wallet/passkey/register/start",
        None,
        cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    let ccr: CreationChallengeResponse = serde_json::from_value(r.body["options"].clone()).unwrap();
    let cred = auth
        .do_registration(Url::parse(WALLET_ORIGIN).unwrap(), ccr)
        .unwrap();
    wallet_call(
        app,
        "POST",
        "/api/v1/wallet/passkey/register/finish",
        Some(json!({"credential": cred})),
        cookies,
    )
    .await
}

/// 用软件 Passkey 完成一次再验证。
pub async fn assert_passkey(app: &Router, auth: &mut SoftAuth, cookies: &[String]) -> Res {
    let r = wallet_call(
        app,
        "POST",
        "/api/v1/wallet/passkey/assert/start",
        None,
        cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    let rcr: RequestChallengeResponse = serde_json::from_value(r.body["options"].clone()).unwrap();
    let cred = auth
        .do_authentication(Url::parse(WALLET_ORIGIN).unwrap(), rcr)
        .unwrap();
    wallet_call(
        app,
        "POST",
        "/api/v1/wallet/passkey/assert/finish",
        Some(json!({"credential": cred})),
        cookies,
    )
    .await
}

/// 开通交易的完整流程：用户名 → 邮箱再验证 → 注册 Passkey → 登记钱包地址。
/// 返回软件 Passkey，会话随之获得 5 分钟近期验证。
pub async fn enable_trading(
    app: &Router,
    db: &PgPool,
    user_id: i64,
    cookies: &[String],
    address: &str,
) -> SoftAuth {
    set_username(db, user_id, &format!("u{user_id}")).await;
    reverify(app, db, cookies).await;
    let mut auth = soft_passkey();
    let r = register_passkey(app, &mut auth, cookies).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    let r = wallet_call(
        app,
        "POST",
        "/api/v1/wallet/account",
        Some(json!({"chain_id": CHAIN_ID, "address": address, "init_version": 1})),
        cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    auth
}

/// 清掉登录验证码的 60 秒重发间隔，让同一用户在测试里马上再登录一次（模拟另一台设备）。
pub async fn clear_resend(email: &str) {
    let url = std::env::var("VALKEY_URL").unwrap();
    let kv = forum_kv::pool(&url).unwrap();
    let tag = CodeStore::new(kv.clone(), &SECRET)
        .unwrap()
        .email_tag(email);
    let mut conn = kv.get().await.unwrap();
    redis::cmd("DEL")
        .arg(format!("auth:resend:{tag}"))
        .query_async::<()>(&mut conn)
        .await
        .unwrap();
}
