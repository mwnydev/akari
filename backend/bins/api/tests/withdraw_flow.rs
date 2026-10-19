//! 提币地址：Passkey 确认、第一个地址立即生效、之后 24 小时冷却、撤销、名额与地址校验、邮件与站内通知。
//! 需要 PostgreSQL（`DATABASE_URL`）与 Valkey（`VALKEY_URL`，未设置则跳过）。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

mod common;

use axum::Router;
use axum::http::StatusCode;
use common::*;
use forum_db::PgPool;
use serde_json::{Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

const WALLET: &str = "0x1111111111111111111111111111111111111111";
const EXCHANGE: &str = "0x2222222222222222222222222222222222222222";
const PATH: &str = "/api/v1/wallet/withdraw-addresses";

fn addr(n: u8) -> String {
    format!("0x{}", hex::encode([n; 20]))
}

async fn add(app: &Router, cookies: &[String], label: &str, address: &str) -> Res {
    wallet_call(
        app,
        "POST",
        PATH,
        Some(json!({"label": label, "address": address})),
        cookies,
    )
    .await
}

fn at(v: &Value) -> OffsetDateTime {
    OffsetDateTime::parse(v.as_str().unwrap(), &Rfc3339).unwrap()
}

/// 开通交易、钱包地址为 `wallet` 的用户（会话带近期 Passkey 验证）。
async fn trader(app: &Router, db: &PgPool, wallet: &str) -> (i64, Vec<String>) {
    let (id, cookies) = user(app, db).await;
    enable_trading(app, db, id, &cookies, wallet).await;
    (id, cookies)
}

async fn outbox(db: &PgPool, kind: &str) -> Vec<Value> {
    sqlx::query_scalar("SELECT payload FROM outbox WHERE kind = $1 ORDER BY id")
        .bind(kind)
        .fetch_all(db)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn first_address_is_immediate_later_ones_cool_down(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let (id, cookies) = trader(&app, &db, WALLET).await;

    let r = add(&app, &cookies, " 币安 ", EXCHANGE).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    assert_eq!(r.body["label"], "币安");
    assert_eq!(r.body["address"], EXCHANGE);
    assert_eq!(r.body["status"], "active");
    let first = r.body["id"].as_i64().unwrap();
    assert!(at(&r.body["active_at"]) <= OffsetDateTime::now_utc());

    // 大小写混合的地址按 EIP-55 校验，存成小写。
    let checksummed = "0x52908400098527886E0F7030069857D2E4169EE7";
    let r = add(&app, &cookies, "OKX", checksummed).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    assert_eq!(r.body["status"], "pending");
    assert_eq!(r.body["address"], checksummed.to_lowercase());
    let wait = at(&r.body["active_at"]) - OffsetDateTime::now_utc();
    assert!(
        wait > time::Duration::hours(23) && wait <= time::Duration::hours(24),
        "{wait}"
    );

    // 邮件与站内通知：写明生效时间；邮件不带用户填的备注。
    let mails = outbox(&db, "email.withdraw_address_added").await;
    assert_eq!(mails.len(), 2);
    assert_eq!(mails[0]["address"], EXCHANGE);
    assert_eq!(mails[0]["immediate"], true);
    assert_eq!(mails[1]["immediate"], false);
    assert!(mails[1]["active_at"].as_str().unwrap().ends_with(" UTC"));
    assert!(mails[0].get("label").is_none());
    let notices = outbox(&db, "notify.withdraw_address_added").await;
    assert_eq!(notices.len(), 2);
    assert_eq!(notices[1]["to"], id);
    assert_eq!(notices[1]["withdraw_address_id"], r.body["id"]);
    assert_eq!(at(&notices[1]["active_at"]), at(&r.body["active_at"]));

    // 列表不要求钱包来源；未撤销的在前、新的在前。
    let r = call(&app, "GET", PATH, None, &cookies).await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    let list = r.body["addresses"].as_array().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0]["status"], "pending");
    assert_eq!(list[1]["id"], first);

    // 撤销第一个后再添加同一地址：不再是第一个地址，要冷却。
    let r = wallet_call(&app, "DELETE", &format!("{PATH}/{first}"), None, &cookies).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{:?}", r.body);
    let r = wallet_call(&app, "DELETE", &format!("{PATH}/{first}"), None, &cookies).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    let r = add(&app, &cookies, "币安", EXCHANGE).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    assert_eq!(r.body["status"], "pending");
    let r = call(&app, "GET", PATH, None, &cookies).await;
    let statuses: Vec<&str> = r.body["addresses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["status"].as_str().unwrap())
        .collect();
    assert_eq!(statuses, ["pending", "pending", "revoked"]);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn adding_and_revoking_need_wallet_origin_and_recent_passkey(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let (id, cookies) = trader(&app, &db, WALLET).await;
    let r = add(&app, &cookies, "", EXCHANGE).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    let created = r.body["id"].as_i64().unwrap();

    let body = json!({"label": "x", "address": addr(0x33)});
    let r = call(&app, "POST", PATH, Some(body.clone()), &cookies).await;
    assert_eq!(r.body["code"], "ORIGIN_REJECTED");
    let r = call(&app, "DELETE", &format!("{PATH}/{created}"), None, &cookies).await;
    assert_eq!(r.body["code"], "ORIGIN_REJECTED");
    let r = wallet_call(&app, "POST", PATH, Some(body.clone()), &[]).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);

    // 另一台设备上的新会话：没有近期 Passkey 验证
    let email: String = sqlx::query_scalar("SELECT email FROM users WHERE id = $1")
        .bind(id)
        .fetch_one(&db)
        .await
        .unwrap();
    clear_resend(&email).await;
    let fresh = jar(&login(&app, &db, &email, &[]).await.cookies);
    let r = wallet_call(&app, "POST", PATH, Some(body), &fresh).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert_eq!(r.body["code"], "PASSKEY_REQUIRED");
    let r = wallet_call(&app, "DELETE", &format!("{PATH}/{created}"), None, &fresh).await;
    assert_eq!(r.body["code"], "PASSKEY_REQUIRED");

    // 别人的地址撤销不了
    let (_, other) = trader(&app, &db, &addr(0x44)).await;
    let r = wallet_call(&app, "DELETE", &format!("{PATH}/{created}"), None, &other).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    let r = call(&app, "GET", PATH, None, &other).await;
    assert_eq!(r.body["addresses"], json!([]));
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn rejects_bad_forbidden_duplicate_and_too_many_addresses(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let (id, cookies) = trader(&app, &db, WALLET).await;

    for bad in [
        "0x1234",
        "2222222222222222222222222222222222222222",
        // 校验和错一位（正确的是 …E4169EE7）
        "0x52908400098527886E0F7030069857D2E4169Ee7",
    ] {
        let r = add(&app, &cookies, "x", bad).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{bad}");
        assert_eq!(r.body["code"], "WITHDRAW_ADDRESS_INVALID", "{bad}");
    }
    // 本人钱包、本站合约、零地址
    for forbidden in [WALLET, ESCROW, USDT, &addr(0)] {
        let r = add(&app, &cookies, "x", forbidden).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{forbidden}");
        assert_eq!(r.body["code"], "WITHDRAW_ADDRESS_FORBIDDEN", "{forbidden}");
    }
    let r = add(&app, &cookies, &"长".repeat(33), EXCHANGE).await;
    assert_eq!(r.body["code"], "INVALID_PARAMETER");

    let r = add(&app, &cookies, "a", EXCHANGE).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    let r = add(
        &app,
        &cookies,
        "b",
        &EXCHANGE.to_uppercase().replace("0X", "0x"),
    )
    .await;
    assert_eq!(r.status, StatusCode::CONFLICT, "{:?}", r.body);

    // 已有 1 个；再加 9 个到上限 10，第 11 个拒绝；撤销一个后又能加。
    sqlx::query(
        "INSERT INTO withdraw_addresses (user_id, chain_id, address, label, active_at)
         SELECT $1, $2, decode(lpad(to_hex(g), 40, '0'), 'hex'), '', now() FROM generate_series(1, 8) g",
    )
    .bind(id)
    .bind(CHAIN_ID as i64)
    .execute(&db)
    .await
    .unwrap();
    let r = add(&app, &cookies, "10th", &addr(0x50)).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    let r = add(&app, &cookies, "11th", &addr(0x51)).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.body["code"], "WITHDRAW_ADDRESS_LIMIT");
    let r = call(&app, "GET", PATH, None, &cookies).await;
    let some = r.body["addresses"][0]["id"].as_i64().unwrap();
    let r = wallet_call(&app, "DELETE", &format!("{PATH}/{some}"), None, &cookies).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    let r = add(&app, &cookies, "11th", &addr(0x51)).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
}
