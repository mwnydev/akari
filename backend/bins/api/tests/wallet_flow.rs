//! 开通交易：邮箱再验证、钱包 Passkey 注册与再验证（软件 Passkey）、钱包地址登记。
//! 需要 PostgreSQL（`DATABASE_URL`）与 Valkey（`VALKEY_URL`，未设置则跳过）。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

mod common;

use axum::http::StatusCode;
use common::{
    CHAIN_ID, WALLET_ORIGIN, assert_passkey, build, call, enable_trading, jar, login,
    register_passkey, reverify, set_username, soft_passkey, unique_email, user, wallet_call,
};
use forum_auth::passkey::webauthn::{CreationChallengeResponse, Url};
use forum_db::PgPool;
use serde_json::json;

const ADDR: &str = "0x1111111111111111111111111111111111111111";

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn wallet_endpoints_reject_forum_origin(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let (_, cookies) = user(&app, &db).await;
    for path in [
        "/api/v1/wallet/reverify/start",
        "/api/v1/wallet/passkey/register/start",
        "/api/v1/wallet/passkey/assert/start",
        "/api/v1/wallet/account",
    ] {
        // 论坛主站来源：通过全局 Origin 白名单，但钱包接口拒绝
        let r = call(&app, "POST", path, Some(json!({})), &cookies).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN, "{path}");
        assert_eq!(r.body["code"], "ORIGIN_REJECTED", "{path}");
    }
    // 未登录
    let r = wallet_call(
        &app,
        "POST",
        "/api/v1/wallet/passkey/register/start",
        None,
        &[],
    )
    .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn first_passkey_needs_username_and_email_reverification(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let (id, cookies) = user(&app, &db).await;
    let start = "/api/v1/wallet/passkey/register/start";

    let r = wallet_call(&app, "POST", start, None, &cookies).await;
    assert_eq!(r.body["code"], "USERNAME_REQUIRED");
    set_username(&db, id, "alice").await;

    // 刚登录也不行：开通交易必须专门做一次邮箱再验证
    let r = wallet_call(&app, "POST", start, None, &cookies).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert_eq!(r.body["code"], "REVERIFY_REQUIRED");

    // 登录验证码不能当开通交易验证码用
    let r = wallet_call(
        &app,
        "POST",
        "/api/v1/wallet/reverify/start",
        None,
        &cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::ACCEPTED);
    let login_code = common::latest_code(&db).await;
    let wallet_code: String = sqlx::query_scalar(
        "SELECT payload->>'code' FROM outbox WHERE kind = 'email.wallet_code' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    if login_code != wallet_code {
        let r = wallet_call(
            &app,
            "POST",
            "/api/v1/wallet/reverify/verify",
            Some(json!({"code": login_code})),
            &cookies,
        )
        .await;
        assert_eq!(r.body["code"], "INVALID_CODE");
    }
    let r = wallet_call(
        &app,
        "POST",
        "/api/v1/wallet/reverify/verify",
        Some(json!({"code": wallet_code})),
        &cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);

    let r = wallet_call(&app, "POST", start, None, &cookies).await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    let pk = &r.body["options"]["publicKey"];
    assert_eq!(pk["rp"]["id"], "wallet.test");
    assert_eq!(pk["user"]["displayName"], "Akari 钱包签名（勿删）· alice");
    assert_eq!(pk["user"]["name"], "Akari 钱包签名（勿删）· alice");
    // 只请求 ES256：Kernel WebAuthn 验证器只认 P-256
    assert_eq!(
        pk["pubKeyCredParams"],
        json!([{"type": "public-key", "alg": -7}])
    );
    assert_eq!(pk["authenticatorSelection"]["userVerification"], "required");
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn register_assert_and_bind_wallet(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let (id, cookies) = user(&app, &db).await;
    set_username(&db, id, "bob").await;
    reverify(&app, &db, &cookies).await;

    let mut auth = soft_passkey();
    let r = register_passkey(&app, &mut auth, &cookies).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    let passkey_id = r.body["id"].as_i64().unwrap();
    assert_eq!(r.body["pubkey_x"].as_str().unwrap().len(), 66);
    assert_eq!(r.body["pubkey_y"].as_str().unwrap().len(), 66);

    let (cose, x, y): (Vec<u8>, Vec<u8>, Vec<u8>) =
        sqlx::query_as("SELECT public_key_cose, pubkey_x, pubkey_y FROM passkeys WHERE id = $1")
            .bind(passkey_id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(cose.len(), 77);
    assert_eq!(&cose[10..42], x.as_slice());
    assert_eq!(&cose[45..], y.as_slice());
    assert_eq!(
        format!("0x{}", hex::encode(&x)),
        r.body["pubkey_x"].as_str().unwrap()
    );

    // 邮箱再验证只能用一次：已有 Passkey 后再注册改为要求近期 Passkey 验证（刚注册即算）
    let r = wallet_call(&app, "GET", "/api/v1/wallet", None, &cookies).await;
    assert_eq!(r.body["enabled"], false, "no wallet address yet");
    assert!(r.body["verified_until"].as_i64().is_some());

    // 链 ID 不对、零地址
    for body in [
        json!({"chain_id": 1, "address": ADDR}),
        json!({"chain_id": CHAIN_ID, "address": "0x0000000000000000000000000000000000000000"}),
    ] {
        let r = wallet_call(&app, "POST", "/api/v1/wallet/account", Some(body), &cookies).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST);
    }
    let body = json!({"chain_id": CHAIN_ID, "address": ADDR});
    let r = wallet_call(
        &app,
        "POST",
        "/api/v1/wallet/account",
        Some(body.clone()),
        &cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    assert_eq!(r.body["passkey_id"], passkey_id);
    assert_eq!(r.body["address"], ADDR);
    // 每人每条链一个
    let other =
        json!({"chain_id": CHAIN_ID, "address": "0x5555555555555555555555555555555555555555"});
    let r = wallet_call(
        &app,
        "POST",
        "/api/v1/wallet/account",
        Some(other),
        &cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::CONFLICT);

    let r = wallet_call(&app, "GET", "/api/v1/wallet", None, &cookies).await;
    assert_eq!(r.body["enabled"], true);
    assert_eq!(r.body["passkeys"].as_array().unwrap().len(), 1);
    assert_eq!(r.body["wallets"][0]["address"], ADDR);

    // 新会话（另一台设备登录）：没有近期验证，绑定 / 再注册都被拒绝
    let email = email_of(&db, id).await;
    common::clear_resend(&email).await;
    let r = login(&app, &db, &email, &[]).await;
    let fresh = jar(&r.cookies);
    let r = wallet_call(
        &app,
        "POST",
        "/api/v1/wallet/passkey/register/start",
        None,
        &fresh,
    )
    .await;
    assert_eq!(r.body["code"], "PASSKEY_REQUIRED");
    let r = wallet_call(&app, "GET", "/api/v1/wallet", None, &fresh).await;
    assert_eq!(r.body["verified_until"], serde_json::Value::Null);

    // 再验证后获得 5 分钟近期验证
    let r = assert_passkey(&app, &mut auth, &fresh).await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert_eq!(r.body["passkey_id"], passkey_id);
    let used: Option<time::OffsetDateTime> =
        sqlx::query_scalar("SELECT last_used_at FROM passkeys WHERE id = $1")
            .bind(passkey_id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert!(used.is_some());
    // 第二台设备的 Passkey：需近期验证，已满足
    let mut second = soft_passkey();
    let r = register_passkey(&app, &mut second, &fresh).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
}

async fn email_of(db: &PgPool, id: i64) -> String {
    sqlx::query_scalar("SELECT email FROM users WHERE id = $1")
        .bind(id)
        .fetch_one(db)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn challenges_are_single_use_and_origin_bound(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let (id, cookies) = user(&app, &db).await;
    set_username(&db, id, "carol").await;
    reverify(&app, &db, &cookies).await;

    // 认证器在别的来源（同 RP ID 的其他子域名）上签出的凭证：服务端只认 WALLET_ORIGINS
    let r = wallet_call(
        &app,
        "POST",
        "/api/v1/wallet/passkey/register/start",
        None,
        &cookies,
    )
    .await;
    let ccr: CreationChallengeResponse = serde_json::from_value(r.body["options"].clone()).unwrap();
    let mut auth = soft_passkey();
    let evil = auth
        .do_registration(Url::parse("https://evil.wallet.test").unwrap(), ccr)
        .unwrap();
    let r = wallet_call(
        &app,
        "POST",
        "/api/v1/wallet/passkey/register/finish",
        Some(json!({"credential": evil})),
        &cookies,
    )
    .await;
    assert_eq!(r.body["code"], "PASSKEY_FAILED");

    // 失败的尝试也消耗了 challenge；再走一遍正常流程（邮箱再验证标记仍在，未被消耗）
    let r = wallet_call(
        &app,
        "POST",
        "/api/v1/wallet/passkey/register/start",
        None,
        &cookies,
    )
    .await;
    let ccr: CreationChallengeResponse = serde_json::from_value(r.body["options"].clone()).unwrap();
    let cred = auth
        .do_registration(Url::parse(WALLET_ORIGIN).unwrap(), ccr)
        .unwrap();
    let body = json!({"credential": cred});
    let r = wallet_call(
        &app,
        "POST",
        "/api/v1/wallet/passkey/register/finish",
        Some(body.clone()),
        &cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    // 重放同一份注册结果：challenge 已用掉
    let r = wallet_call(
        &app,
        "POST",
        "/api/v1/wallet/passkey/register/finish",
        Some(body),
        &cookies,
    )
    .await;
    assert_eq!(r.body["code"], "PASSKEY_FAILED");

    // 断言同理：重放被拒
    let r = wallet_call(
        &app,
        "POST",
        "/api/v1/wallet/passkey/assert/start",
        None,
        &cookies,
    )
    .await;
    let rcr = serde_json::from_value(r.body["options"].clone()).unwrap();
    let cred = auth
        .do_authentication(Url::parse(WALLET_ORIGIN).unwrap(), rcr)
        .unwrap();
    let body = json!({"credential": cred});
    for expected in [StatusCode::OK, StatusCode::BAD_REQUEST] {
        let r = wallet_call(
            &app,
            "POST",
            "/api/v1/wallet/passkey/assert/finish",
            Some(body.clone()),
            &cookies,
        )
        .await;
        assert_eq!(r.status, expected, "{:?}", r.body);
    }
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn assert_needs_a_passkey_and_addresses_are_unique(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let (_, nobody) = user(&app, &db).await;
    let r = wallet_call(
        &app,
        "POST",
        "/api/v1/wallet/passkey/assert/start",
        None,
        &nobody,
    )
    .await;
    assert_eq!(r.body["code"], "WALLET_REQUIRED");
    let r = wallet_call(
        &app,
        "POST",
        "/api/v1/wallet/account",
        Some(json!({"chain_id": CHAIN_ID, "address": ADDR})),
        &nobody,
    )
    .await;
    assert_eq!(r.body["code"], "PASSKEY_REQUIRED");

    let (a, ca) = user(&app, &db).await;
    enable_trading(&app, &db, a, &ca, ADDR).await;
    // 同一地址不能被第二个人登记
    let r = login(&app, &db, &unique_email(), &[]).await;
    let b = r.body["user"]["id"].as_i64().unwrap();
    let cb = jar(&r.cookies);
    set_username(&db, b, "dave").await;
    reverify(&app, &db, &cb).await;
    let mut auth = soft_passkey();
    assert_eq!(
        register_passkey(&app, &mut auth, &cb).await.status,
        StatusCode::CREATED
    );
    let r = wallet_call(
        &app,
        "POST",
        "/api/v1/wallet/account",
        Some(json!({"chain_id": CHAIN_ID, "address": ADDR})),
        &cb,
    )
    .await;
    assert_eq!(r.status, StatusCode::CONFLICT);
}
