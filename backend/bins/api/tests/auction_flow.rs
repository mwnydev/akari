//! 拍卖：开关、挂拍校验、签名上线、出价规则（卖家与关联账号、新账号上限、出价资格、最低加价）、并发出价、
//! 防狙击顺延与上限、出价记录打码、被超价通知、取消、主题列表信息行、中标者付款数据。
//! 验签与余额用假的链上读取（[`common::FakeAuctionChain`]）；真实链上验签、成交证明与 `createFromAuction`
//! 见 `auction_e2e.rs`。需要 PostgreSQL（`DATABASE_URL`）与 Valkey（`VALKEY_URL`，未设置则跳过）。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

mod common;

use std::sync::Arc;

use axum::Router;
use axum::http::StatusCode;
use common::{
    ESCROW, FakeAuctionChain, Res, VALID_AUCTION_SIGNATURE, age, build, build_with, call,
    enable_trading, user, wallet_state,
};
use forum_core::auction::AuctionSettings;
use forum_core::eip712::{AuctionAuth, Domain};
use forum_db::PgPool;
use forum_db::forums::{self, NewForum};
use serde_json::{Value, json};

const U: u128 = 1_000_000;
const SELLER: &str = "0x1111111111111111111111111111111111111111";

async fn market(db: &PgPool) -> i32 {
    forums::create(
        db,
        &NewForum {
            parent_id: None,
            slug: "market",
            name: "market",
            description: "",
            position: 0,
            view_min_level: 0,
            post_min_level: 1,
            reply_min_level: 1,
            allow_trade: true,
            allow_auction: true,
            color: None,
        },
    )
    .await
    .unwrap()
}

async fn switch(db: &PgPool, enabled: bool) {
    let by: i64 = sqlx::query_scalar("SELECT min(id) FROM users")
        .fetch_one(db)
        .await
        .unwrap();
    forum_db::settings::set_auction(
        db,
        &AuctionSettings {
            enabled,
            ..AuctionSettings::default()
        },
        by,
    )
    .await
    .unwrap();
}

struct Party {
    id: i64,
    cookies: Vec<String>,
}

/// 卖家：完整开通交易（会话带 5 分钟近期 Passkey 验证）。
async fn seller(app: &Router, db: &PgPool) -> Party {
    let (id, cookies) = user(app, db).await;
    age(db, id).await;
    enable_trading(app, db, id, &cookies, SELLER).await;
    Party { id, cookies }
}

/// 出价人：登录 + 直接登记钱包（出价不需要 Passkey）。`aged` 为假时是刚注册的新账号。
async fn bidder(app: &Router, db: &PgPool, n: u8, aged: bool) -> Party {
    let (id, cookies) = user(app, db).await;
    if aged {
        sqlx::query("UPDATE users SET created_at = now() - interval '30 days' WHERE id = $1")
            .bind(id)
            .execute(db)
            .await
            .unwrap();
    }
    sqlx::query("UPDATE users SET username = $2 WHERE id = $1")
        .bind(id)
        .bind(format!("bidder{n:02}x"))
        .execute(db)
        .await
        .unwrap();
    let pk: i64 = sqlx::query_scalar(
        "INSERT INTO passkeys (user_id, purpose, credential_id, credential) VALUES ($1, 2, $2, '{}') RETURNING id",
    )
    .bind(id)
    .bind(format!("pk-{id}").into_bytes())
    .fetch_one(db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO wallets (user_id, chain_id, address, passkey_id) VALUES ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(common::CHAIN_ID as i64)
    .bind(address(n).to_vec())
    .bind(pk)
    .execute(db)
    .await
    .unwrap();
    Party { id, cookies }
}

fn address(n: u8) -> [u8; 20] {
    [
        0xb0 + (n % 0x40),
        n,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        n,
    ]
}

fn address_hex(n: u8) -> String {
    format!("0x{}", hex::encode(address(n)))
}

fn usdt_auction(start: u128, increment: u128, reserve: u128) -> Value {
    json!({
        "title": "限量版键帽",
        "html": "<p>全新未拆</p>",
        "source": {"type": "doc"},
        "currency": "usdt",
        "start_price": start.to_string(),
        "increment": increment.to_string(),
        "reserve_price": reserve.to_string(),
        "duration_secs": 86_400,
        "terms": {"ship_window": 3 * 86_400, "confirm_window": 7 * 86_400}
    })
}

async fn create(app: &Router, s: &Party, body: Value) -> Res {
    call(
        app,
        "POST",
        "/api/v1/forums/market/auctions",
        Some(body),
        &s.cookies,
    )
    .await
}

/// 建草稿、开始签名、提交（假的）有效签名，返回上线后的拍卖。
async fn live(app: &Router, s: &Party, body: Value) -> Value {
    let r = create(app, s, body).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    let id = r.body["id"].as_i64().unwrap();
    let r = call(
        app,
        "POST",
        &format!("/api/v1/auctions/{id}/start"),
        None,
        &s.cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    let r = call(
        app,
        "POST",
        &format!("/api/v1/auctions/{id}/finish"),
        Some(json!({"signature": VALID_AUCTION_SIGNATURE})),
        &s.cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    r.body
}

async fn bid(app: &Router, id: i64, p: &Party, amount: u128) -> Res {
    call(
        app,
        "POST",
        &format!("/api/v1/auctions/{id}/bids"),
        Some(json!({"amount": amount.to_string()})),
        &p.cookies,
    )
    .await
}

fn code(r: &Res) -> &str {
    r.body["code"].as_str().unwrap_or_default()
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn switched_off_by_default(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    market(&db).await;
    let s = seller(&app, &db).await;
    let r = call(&app, "GET", "/api/v1/auctions/params", None, &[]).await;
    assert_eq!(r.body["settings"]["enabled"], false);
    let r = create(&app, &s, usdt_auction(10 * U, U, 20 * U)).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert_eq!(code(&r), "AUCTIONS_DISABLED");

    switch(&db, true).await;
    let a = live(&app, &s, usdt_auction(10 * U, U, 20 * U)).await;
    let id = a["id"].as_i64().unwrap();
    let b = bidder(&app, &db, 1, true).await;
    // 关掉之后不能再出价、取消；详情照常可看。
    switch(&db, false).await;
    assert_eq!(code(&bid(&app, id, &b, 10 * U).await), "AUCTIONS_DISABLED");
    let cancel = call(
        &app,
        "POST",
        &format!("/api/v1/auctions/{id}/cancel"),
        None,
        &s.cookies,
    )
    .await;
    assert_eq!(code(&cancel), "AUCTIONS_DISABLED");
    let r = call(&app, "GET", &format!("/api/v1/auctions/{id}"), None, &[]).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.body["state"], "live");
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn only_auction_boards_take_auctions(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let market_id = market(&db).await;
    // 只允许交易帖的板块不能挂拍
    forums::create(
        &db,
        &NewForum {
            parent_id: None,
            slug: "trade",
            name: "trade",
            description: "",
            position: 1,
            view_min_level: 0,
            post_min_level: 1,
            reply_min_level: 1,
            allow_trade: true,
            allow_auction: false,
            color: None,
        },
    )
    .await
    .unwrap();
    let s = seller(&app, &db).await;
    switch(&db, true).await;
    let r = call(
        &app,
        "POST",
        "/api/v1/forums/trade/auctions",
        Some(usdt_auction(10 * U, U, 20 * U)),
        &s.cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert_eq!(code(&r), "AUCTION_NOT_ALLOWED");
    let r = call(&app, "GET", "/api/v1/forums", None, &[]).await;
    let flags: Vec<(Value, Value)> = r
        .body
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["slug"].clone(), f["allow_auction"].clone()))
        .collect();
    assert_eq!(
        flags,
        [
            (json!("market"), json!(true)),
            (json!("trade"), json!(false))
        ]
    );

    // 草稿建好之后站长关掉了拍卖：签名后也不能上线，不发帖
    let r = create(&app, &s, usdt_auction(10 * U, U, 20 * U)).await;
    let id = r.body["id"].as_i64().unwrap();
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/auctions/{id}/start"),
        None,
        &s.cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    sqlx::query("UPDATE forums SET allow_auction = false WHERE id = $1")
        .bind(market_id)
        .execute(&db)
        .await
        .unwrap();
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/auctions/{id}/finish"),
        Some(json!({"signature": VALID_AUCTION_SIGNATURE})),
        &s.cookies,
    )
    .await;
    assert_eq!(code(&r), "AUCTION_NOT_ALLOWED");
    let topics: i64 = sqlx::query_scalar("SELECT count(*) FROM topics")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(topics, 0);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn listing_terms_are_validated(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    market(&db).await;
    let s = seller(&app, &db).await;
    switch(&db, true).await;
    let cases = [
        // 加价幅度低于 1 USDT
        (
            usdt_auction(10 * U, U - 1, 20 * U),
            "AUCTION_INCREMENT_TOO_LOW",
        ),
        // 低于起拍价的 1%
        (
            usdt_auction(150 * U, U, 150 * U),
            "AUCTION_INCREMENT_TOO_LOW",
        ),
        // 保留价低于起拍价
        (usdt_auction(10 * U, U, 9 * U), "INVALID_PARAMETER"),
        // 起拍价低于订单下限 5 USDT、保留价高于订单上限 200 USDT
        (usdt_auction(4 * U, U, 10 * U), "AMOUNT_OUT_OF_RANGE"),
        (usdt_auction(10 * U, U, 201 * U), "AMOUNT_OUT_OF_RANGE"),
    ];
    for (body, expect) in cases {
        let r = create(&app, &s, body.clone()).await;
        assert_eq!(code(&r), expect, "{body}");
    }
    let mut short = usdt_auction(10 * U, U, 20 * U);
    short["duration_secs"] = json!(3600);
    assert_eq!(
        code(&create(&app, &s, short).await),
        "AUCTION_DURATION_INVALID"
    );
    let mut bad_eligibility = usdt_auction(10 * U, U, 20 * U);
    bad_eligibility["eligibility"] = json!({"kind": "percent", "percent": 20});
    assert_eq!(
        code(&create(&app, &s, bad_eligibility).await),
        "INVALID_PARAMETER"
    );
    let mut window = usdt_auction(10 * U, U, 20 * U);
    window["terms"]["ship_window"] = json!(3600);
    assert_eq!(code(&create(&app, &s, window).await), "WINDOW_OUT_OF_RANGE");

    // 人民币：必须带汇率；1 USDT 按汇率 7.2 折 720 分，加价幅度不能更低。
    let cny = |rate: Value, start: u32, increment: u32, reserve: u32| {
        let mut b = usdt_auction(0, 0, 0);
        b["currency"] = json!("cny");
        b["cny_per_usdt"] = rate;
        b["start_price"] = json!(start.to_string());
        b["increment"] = json!(increment.to_string());
        b["reserve_price"] = json!(reserve.to_string());
        b
    };
    assert_eq!(
        code(&create(&app, &s, cny(Value::Null, 5000, 720, 9000)).await),
        "INVALID_PARAMETER"
    );
    assert_eq!(
        code(&create(&app, &s, cny(json!(72), 5000, 720, 9000)).await),
        "INVALID_PARAMETER"
    );
    assert_eq!(
        code(&create(&app, &s, cny(json!(72_000), 5000, 719, 9000)).await),
        "AUCTION_INCREMENT_TOO_LOW"
    );
    // 36 元 = 5 USDT 整，刚好到订单下限
    assert_eq!(
        code(&create(&app, &s, cny(json!(72_000), 3599, 720, 9000)).await),
        "AMOUNT_OUT_OF_RANGE"
    );
    let r = create(&app, &s, cny(json!(72_000), 3600, 720, 9000)).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    assert_eq!(r.body["state"], "draft");
    assert_eq!(r.body["currency"], "cny");
    assert_eq!(r.body["current_usdt"], "5000000");
    assert_eq!(
        r.body["reserve_price"], "9000",
        "the seller sees the reserve"
    );

    // 草稿只有卖家看得到
    let id = r.body["id"].as_i64().unwrap();
    let other = bidder(&app, &db, 1, true).await;
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/auctions/{id}"),
        None,
        &other.cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn seller_signs_the_authorization_before_going_live(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    market(&db).await;
    let s = seller(&app, &db).await;
    switch(&db, true).await;
    let r = create(&app, &s, usdt_auction(10 * U, 2 * U, 25 * U)).await;
    let id = r.body["id"].as_i64().unwrap();
    // 还没开始签名就提交：不可用
    let finish = |sig: &'static str| {
        let (app, c) = (app.clone(), s.cookies.clone());
        async move {
            call(
                &app,
                "POST",
                &format!("/api/v1/auctions/{id}/finish"),
                Some(json!({"signature": sig})),
                &c,
            )
            .await
        }
    };
    assert_eq!(
        code(&finish(VALID_AUCTION_SIGNATURE).await),
        "AUCTION_UNAVAILABLE"
    );

    let r = call(
        &app,
        "POST",
        &format!("/api/v1/auctions/{id}/start"),
        None,
        &s.cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    let td = &r.body["typed_data"];
    assert_eq!(td["primaryType"], "AuctionAuth");
    assert_eq!(td["domain"]["name"], "AkariEscrow");
    assert_eq!(td["domain"]["verifyingContract"], ESCROW);
    assert_eq!(td["types"]["AuctionAuth"].as_array().unwrap().len(), 15);
    let msg = &td["message"];
    assert_eq!(msg["seller"], SELLER);
    assert_eq!(msg["auctionId"], id.to_string());
    assert_eq!(msg["reservePrice"], (25 * U).to_string());
    assert_eq!(msg["currency"], 0);
    assert_eq!(msg["cnyPerUsdt"], 0);
    assert_eq!(msg["maxFeeBps"], 100);
    // 有效期覆盖：签名时限 + 时长 + 顺延上限 + 付款期 + 两轮递补
    let now = time::OffsetDateTime::now_utc().unix_timestamp() as u64;
    let d = AuctionSettings::default();
    let min_expiry = now
        + 86_400
        + u64::from(d.extension_cap_secs + d.pay_window_secs + 2 * d.runner_up_window_secs);
    assert!(msg["expiry"].as_u64().unwrap() > min_expiry);
    // 返回的摘要就是 hashAuction(message)
    let auth: AuctionAuth = serde_json::from_value(msg.clone()).unwrap();
    let domain = Domain {
        chain_id: common::CHAIN_ID,
        verifying_contract: ESCROW.parse().unwrap(),
    };
    assert_eq!(r.body["auth_hash"], auth.hash(&domain).to_string());

    // 签名不对：拒绝；仍是草稿
    assert_eq!(code(&finish("0xdead").await), "AUCTION_SIGNATURE_INVALID");
    // 签名超时：要重新开始
    sqlx::query("UPDATE auctions SET sign_deadline = now() - interval '1 second' WHERE id = $1")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(
        code(&finish(VALID_AUCTION_SIGNATURE).await),
        "AUCTION_SIGN_EXPIRED"
    );
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/auctions/{id}/start"),
        None,
        &s.cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    let r = finish(VALID_AUCTION_SIGNATURE).await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert_eq!(r.body["state"], "live");
    assert_eq!(r.body["bid_count"], 0);
    assert_eq!(r.body["min_next_bid"], (10 * U).to_string());
    let topic_id = r.body["topic_id"].as_i64().unwrap();
    let ends_at = time::OffsetDateTime::parse(
        r.body["ends_at"].as_str().unwrap(),
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap();
    assert!((ends_at.unix_timestamp() - now as i64 - 86_400).abs() < 60);
    // 已上线：不能再签
    assert_eq!(
        code(&finish(VALID_AUCTION_SIGNATURE).await),
        "AUCTION_UNAVAILABLE"
    );

    // 拍卖帖：主题页取拍卖、主题列表带信息行（游客也能看）
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/topics/{topic_id}/auction"),
        None,
        &[],
    )
    .await;
    assert_eq!(r.body["id"], id);
    assert_eq!(
        r.body["reserve_price"],
        Value::Null,
        "the reserve stays hidden"
    );
    assert_eq!(r.body["my"], Value::Null);
    let r = call(&app, "GET", "/api/v1/forums/market/topics", None, &[]).await;
    let row = r.body["topics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == topic_id)
        .unwrap();
    assert_eq!(row["kind"], 3);
    assert_eq!(row["auction"]["auction_id"], id);
    assert_eq!(row["auction"]["state"], "live");
    assert_eq!(row["auction"]["currency"], "usdt");
    assert_eq!(row["auction"]["current"], (10 * U).to_string());
    assert_eq!(row["auction"]["bids"], 0);
    assert_eq!(row["auction"]["reserve_met"], false);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn bidding_rules(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    market(&db).await;
    let s = seller(&app, &db).await;
    switch(&db, true).await;
    let a = live(&app, &s, usdt_auction(10 * U, 2 * U, 25 * U)).await;
    let id = a["id"].as_i64().unwrap();
    let topic_id = a["topic_id"].as_i64().unwrap();
    // 游客的板块列表先进缓存：之后的出价必须让它失效（见下面「主题列表信息行跟着变」）
    let r = call(&app, "GET", "/api/v1/forums/market/topics", None, &[]).await;
    assert_eq!(r.body["topics"][0]["auction"]["bids"], 0, "{:?}", r.body);
    let r = call(&app, "GET", "/api/v1/forums/market/topics", None, &[]).await;
    assert_eq!(r.headers["x-guest-cache"], "hit");

    // 卖家本人不能出价；与卖家登录过同一台设备的账号也不能
    assert_eq!(code(&bid(&app, id, &s, 10 * U).await), "BID_BY_SELLER");
    let alt = bidder(&app, &db, 9, true).await;
    for u in [s.id, alt.id] {
        sqlx::query("INSERT INTO user_devices (user_id, device_hash) VALUES ($1, $2) ON CONFLICT DO NOTHING")
            .bind(u)
            .bind(vec![7u8; 32])
            .execute(&db)
            .await
            .unwrap();
    }
    assert_eq!(code(&bid(&app, id, &alt, 10 * U).await), "BID_BY_SELLER");

    let a1 = bidder(&app, &db, 1, true).await;
    let a2 = bidder(&app, &db, 2, true).await;
    assert_eq!(code(&bid(&app, id, &a1, 10 * U - 1).await), "BID_TOO_LOW");
    let r = bid(&app, id, &a1, 10 * U).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    assert_eq!(r.body["bid_count"], 1);
    assert_eq!(r.body["my"]["leading"], true);
    // 加价幅度 2 USDT：下一口最低 12 USDT
    assert_eq!(r.body["min_next_bid"], (12 * U).to_string());
    assert_eq!(
        code(&bid(&app, id, &a1, 20 * U).await),
        "BID_ALREADY_LEADING"
    );
    assert_eq!(code(&bid(&app, id, &a2, 12 * U - 1).await), "BID_TOO_LOW");
    let r = bid(&app, id, &a2, 30 * U).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    assert_eq!(r.body["reserve_met"], true);
    // 30 USDT 的 1% 不到 2 USDT，仍按幅度
    assert_eq!(r.body["min_next_bid"], (32 * U).to_string());
    // 订单上限 200 USDT 之外的出价无法成交
    assert_eq!(
        code(&bid(&app, id, &a1, 201 * U).await),
        "AMOUNT_OUT_OF_RANGE"
    );

    // 新账号（注册不满 7 天）单场出价最多 100 USDT
    let young = bidder(&app, &db, 3, false).await;
    assert_eq!(
        code(&bid(&app, id, &young, 101 * U).await),
        "BID_NEW_ACCOUNT_CAP"
    );
    let r = bid(&app, id, &young, 100 * U).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    // 100 USDT 之后的最低加价是 1%（1 USDT）与幅度取大
    assert_eq!(r.body["min_next_bid"], (102 * U).to_string());

    // 被超价的人各收到一条通知（a1 被 a2 超过，a2 被 young 超过）
    let outbid: Vec<(i64, String)> = sqlx::query_as(
        "SELECT (payload->>'to')::bigint, payload->>'amount' FROM outbox WHERE kind = 'notify.auction'
           AND payload->>'event' = 'outbid' ORDER BY id",
    )
    .fetch_all(&db)
    .await
    .unwrap();
    assert_eq!(
        outbid,
        [
            (a1.id, (30 * U).to_string()),
            (a2.id, (100 * U).to_string())
        ]
    );

    // 出价记录：新的在前、出价人打码、标出自己的
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/auctions/{id}/bids"),
        None,
        &a1.cookies,
    )
    .await;
    let bids = r.body["bids"].as_array().unwrap();
    assert_eq!(bids.len(), 3);
    assert_eq!(bids[0]["amount"], (100 * U).to_string());
    assert_eq!(bids[2]["bidder"], "b***x");
    assert_eq!(bids[2]["mine"], true);
    assert_eq!(bids[0]["mine"], false);
    assert!(
        bids.iter()
            .all(|b| !b["bidder"].as_str().unwrap().contains("bidder"))
    );
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/auctions/{id}"),
        None,
        &a1.cookies,
    )
    .await;
    assert_eq!(r.body["my"]["outbid"], true);
    assert_eq!(r.body["my"]["best_bid"], (10 * U).to_string());
    assert_eq!(r.body["current_price"], (100 * U).to_string());

    // 两个出价人是关联账号（同设备）：后出价的那口被标出来
    for u in [a1.id, a2.id] {
        sqlx::query("INSERT INTO user_devices (user_id, device_hash) VALUES ($1, $2)")
            .bind(u)
            .bind(vec![8u8; 32])
            .execute(&db)
            .await
            .unwrap();
    }
    let r = bid(&app, id, &a1, 102 * U).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/auctions/{id}/bids"),
        None,
        &[],
    )
    .await;
    assert_eq!(r.body["bids"][0]["linked"], true);
    assert_eq!(r.body["bids"][1]["linked"], false);

    // 主题列表信息行跟着变（游客缓存已被出价失效）
    let r = call(&app, "GET", "/api/v1/forums/market/topics", None, &[]).await;
    assert_eq!(r.headers["x-guest-cache"], "miss");
    let row = r.body["topics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == topic_id)
        .unwrap()
        .clone();
    assert_eq!(row["auction"]["bids"], 4);
    assert_eq!(row["auction"]["current"], (102 * U).to_string());
    assert_eq!(row["auction"]["reserve_met"], true);

    // 结束之后不能再出价
    sqlx::query("UPDATE auctions SET ends_at = now() - interval '1 second' WHERE id = $1")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(
        code(&bid(&app, id, &a2, 200 * U).await),
        "AUCTION_UNAVAILABLE"
    );
    let r = call(&app, "GET", &format!("/api/v1/auctions/{id}"), None, &[]).await;
    assert_eq!(r.body["state"], "ended");
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn eligibility_reads_the_wallet_balance(db: PgPool) {
    let chain = Arc::new(FakeAuctionChain::default());
    let wallet = forum_api::wallet::WalletState {
        auction_chain: chain.clone(),
        ..wallet_state()
    };
    let Some(app) = build_with(db.clone(), wallet) else {
        return;
    };
    market(&db).await;
    let s = seller(&app, &db).await;
    switch(&db, true).await;
    let mut body = usdt_auction(10 * U, U, 10 * U);
    body["eligibility"] = json!({"kind": "percent", "percent": 50});
    let a = live(&app, &s, body).await;
    let id = a["id"].as_i64().unwrap();
    assert_eq!(a["eligibility"], json!({"kind": "percent", "percent": 50}));
    let b = bidder(&app, &db, 1, true).await;
    // 出价 20 USDT 要求余额 ≥ 10 USDT
    chain.set_balance(&address_hex(1), 10 * U - 1);
    assert_eq!(
        code(&bid(&app, id, &b, 20 * U).await),
        "BID_BALANCE_INSUFFICIENT"
    );
    // 余额缓存几秒：换一个钱包的出价人验证新余额
    let c = bidder(&app, &db, 2, true).await;
    chain.set_balance(&address_hex(2), 10 * U);
    let r = bid(&app, id, &c, 20 * U).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);

    // 固定金额门槛
    let mut body = usdt_auction(10 * U, U, 10 * U);
    body["eligibility"] = json!({"kind": "fixed", "usdt": (50 * U).to_string()});
    let a = live(&app, &s, body).await;
    let id = a["id"].as_i64().unwrap();
    assert_eq!(
        code(&bid(&app, id, &c, 10 * U).await),
        "BID_BALANCE_INSUFFICIENT"
    );
    let d = bidder(&app, &db, 3, true).await;
    chain.set_balance(&address_hex(3), 50 * U);
    assert_eq!(bid(&app, id, &d, 10 * U).await.status, StatusCode::CREATED);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn anti_snipe_extends_up_to_the_cap(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    market(&db).await;
    let s = seller(&app, &db).await;
    switch(&db, true).await;
    let a = live(&app, &s, usdt_auction(10 * U, U, 10 * U)).await;
    let id = a["id"].as_i64().unwrap();
    // 防狙击参数随拍卖给出（开始签名时取自站长设置），前端据此显示规则与已顺延的时长
    assert_eq!(
        (
            &a["duration_secs"],
            &a["snipe_window_secs"],
            &a["snipe_extension_secs"],
            &a["extension_cap_secs"]
        ),
        (&json!(86_400), &json!(900), &json!(900), &json!(86_400)),
        "{a:?}"
    );
    let (b1, b2) = (
        bidder(&app, &db, 1, true).await,
        bidder(&app, &db, 2, true).await,
    );
    let ends = || {
        let db = db.clone();
        async move {
            sqlx::query_as::<_, (i64, i64)>(
                "SELECT extract(epoch FROM ends_at)::bigint, extract(epoch FROM max_ends_at)::bigint
                 FROM auctions WHERE id = $1",
            )
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap()
        }
    };
    // 窗口（结束前 15 分钟）之外：不顺延
    let (before, _) = ends().await;
    assert_eq!(bid(&app, id, &b1, 10 * U).await.status, StatusCode::CREATED);
    assert_eq!(ends().await.0, before);

    // 窗口之内：顺延 15 分钟
    sqlx::query("UPDATE auctions SET ends_at = now() + interval '5 minutes' WHERE id = $1")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    let (before, _) = ends().await;
    let r = bid(&app, id, &b2, 11 * U).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    assert_eq!(ends().await.0, before + 900);

    // 累计顺延到上限为止
    sqlx::query(
        "UPDATE auctions SET ends_at = now() + interval '2 minutes',
                max_ends_at = now() + interval '7 minutes' WHERE id = $1",
    )
    .bind(id)
    .execute(&db)
    .await
    .unwrap();
    assert_eq!(bid(&app, id, &b1, 12 * U).await.status, StatusCode::CREATED);
    let (end, cap) = ends().await;
    assert_eq!(end, cap);
    // 已到上限：再出价也不再顺延
    assert_eq!(bid(&app, id, &b2, 13 * U).await.status, StatusCode::CREATED);
    assert_eq!(ends().await, (cap, cap));
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn concurrent_bids_only_valid_increments_win(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    market(&db).await;
    let s = seller(&app, &db).await;
    switch(&db, true).await;
    let a = live(&app, &s, usdt_auction(10 * U, U, 10 * U)).await;
    let id = a["id"].as_i64().unwrap();
    let mut bidders = Vec::new();
    for n in 1..=12 {
        bidders.push(Arc::new(bidder(&app, &db, n, true).await));
    }

    // 所有人同时出起拍价：只有一口成功，其余都是出价太低。
    let tasks: Vec<_> = bidders
        .iter()
        .map(|b| {
            let (app, b) = (app.clone(), b.clone());
            tokio::spawn(async move { bid(&app, id, &b, 10 * U).await })
        })
        .collect();
    let mut ok = 0;
    for t in tasks {
        let r = t.await.unwrap();
        match r.status {
            StatusCode::CREATED => ok += 1,
            StatusCode::CONFLICT => assert_eq!(code(&r), "BID_TOO_LOW", "{:?}", r.body),
            other => panic!("unexpected {other}: {:?}", r.body),
        }
    }
    assert_eq!(ok, 1);

    // 同时出不同的价：成功的那些按出价顺序严格递增、且每口都不低于上一口 + 最低加价。
    let tasks: Vec<_> = bidders
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let (app, b) = (app.clone(), b.clone());
            let amount = (11 + i as u128 * 2) * U;
            tokio::spawn(async move { bid(&app, id, &b, amount).await })
        })
        .collect();
    for t in tasks {
        let r = t.await.unwrap();
        assert!(
            r.status == StatusCode::CREATED || r.status == StatusCode::CONFLICT,
            "{:?}",
            r.body
        );
    }
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT amount::text, bidder_id FROM auction_bids WHERE auction_id = $1 ORDER BY id",
    )
    .bind(id)
    .fetch_all(&db)
    .await
    .unwrap();
    let amounts: Vec<u128> = rows.iter().map(|(a, _)| a.parse().unwrap()).collect();
    for w in amounts.windows(2) {
        assert!(w[1] >= w[0] + U.max(w[0].div_ceil(100)), "{amounts:?}");
    }
    for w in rows.windows(2) {
        assert_ne!(w[0].1, w[1].1, "nobody outbids themselves");
    }
    let (count, top): (i32, String) =
        sqlx::query_as("SELECT bid_count, top_amount::text FROM auctions WHERE id = $1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(count as usize, rows.len());
    assert_eq!(top.parse::<u128>().unwrap(), *amounts.last().unwrap());
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn seller_cancels_only_before_the_first_bid(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    market(&db).await;
    let s = seller(&app, &db).await;
    switch(&db, true).await;
    let cancel = |id: i64, c: Vec<String>| {
        let app = app.clone();
        async move {
            call(
                &app,
                "POST",
                &format!("/api/v1/auctions/{id}/cancel"),
                None,
                &c,
            )
            .await
        }
    };
    let draft = create(&app, &s, usdt_auction(10 * U, U, 10 * U)).await;
    let r = cancel(draft.body["id"].as_i64().unwrap(), s.cookies.clone()).await;
    assert_eq!(r.body["state"], "cancelled", "{:?}", r.body);

    let a = live(&app, &s, usdt_auction(10 * U, U, 10 * U)).await;
    let id = a["id"].as_i64().unwrap();
    let b = bidder(&app, &db, 1, true).await;
    assert_eq!(
        cancel(id, b.cookies.clone()).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(bid(&app, id, &b, 10 * U).await.status, StatusCode::CREATED);
    assert_eq!(
        code(&cancel(id, s.cookies.clone()).await),
        "AUCTION_HAS_BIDS"
    );

    let a = live(&app, &s, usdt_auction(10 * U, U, 10 * U)).await;
    let id = a["id"].as_i64().unwrap();
    let board = |app: Router| async move {
        let r = call(&app, "GET", "/api/v1/forums/market/topics", None, &[]).await;
        r.body["topics"][0]["auction"]["state"].clone()
    };
    // 上线让游客列表失效，取消也一样
    assert_eq!(board(app.clone()).await, "live");
    let r = cancel(id, s.cookies.clone()).await;
    assert_eq!(r.body["state"], "cancelled");
    assert_eq!(board(app.clone()).await, "cancelled");
    assert_eq!(
        code(&bid(&app, id, &b, 10 * U).await),
        "AUCTION_UNAVAILABLE"
    );
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn payment_data_is_only_for_the_current_winner(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    market(&db).await;
    let s = seller(&app, &db).await;
    switch(&db, true).await;
    // 人民币拍卖，卖家汇率 7.2：成交价 57,601 分 → ceil(57601 × 10^8 ÷ 72000) = 80,001,389 USDT 最小单位
    let mut body = usdt_auction(0, 0, 0);
    body["currency"] = json!("cny");
    body["cny_per_usdt"] = json!(72_000);
    body["start_price"] = json!("50000");
    body["increment"] = json!("1000");
    body["reserve_price"] = json!("50000");
    let a = live(&app, &s, body).await;
    let id = a["id"].as_i64().unwrap();
    let (w, l) = (
        bidder(&app, &db, 1, true).await,
        bidder(&app, &db, 2, true).await,
    );
    assert_eq!(bid(&app, id, &l, 50_000).await.status, StatusCode::CREATED);
    let r = bid(&app, id, &w, 57_601).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    assert_eq!(r.body["current_usdt"], "80001389");

    let payment = |c: Vec<String>| {
        let app = app.clone();
        async move {
            call(
                &app,
                "GET",
                &format!("/api/v1/auctions/{id}/payment"),
                None,
                &c,
            )
            .await
        }
    };
    assert_eq!(
        code(&payment(w.cookies.clone()).await),
        "AUCTION_UNAVAILABLE"
    );

    // forum-sponsor 发的证明（这里直接写库）：第 1 轮给 w。
    let (bid_id, auth_hash): (i64, Vec<u8>) = sqlx::query_as(
        "SELECT b.id, a.auth_hash FROM auction_bids b JOIN auctions a ON a.id = b.auction_id
         WHERE b.auction_id = $1 AND b.bidder_id = $2",
    )
    .bind(id)
    .bind(w.id)
    .fetch_one(&db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO auction_proofs (auction_id, round, bid_id, bidder_id, auth_hash, winner, price, pay_deadline, signature)
         VALUES ($1, 1, $2, $3, $4, $5, 57601, now() + interval '48 hours', '\\x01')",
    )
    .bind(id)
    .bind(bid_id)
    .bind(w.id)
    .bind(&auth_hash)
    .bind(address(1).to_vec())
    .execute(&db)
    .await
    .unwrap();
    sqlx::query("UPDATE auctions SET state = 2, round = 1, pay_deadline = now() + interval '48 hours' WHERE id = $1")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();

    let r = payment(w.cookies.clone()).await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert_eq!(r.body["round"], 1);
    assert_eq!(r.body["price"], "57601");
    assert_eq!(r.body["amount"], "80001389");
    assert_eq!(r.body["currency"], "cny");
    assert_eq!(r.body["proof"]["winner"], address_hex(1));
    assert_eq!(r.body["proof"]["price"], "57601");
    assert_eq!(r.body["auth"]["cnyPerUsdt"], 72_000);
    assert_eq!(r.body["buyer_wallet"], address_hex(1));
    // 卖家承担手续费：买家只付金额 + 0（gas 由卖家方承担）
    assert_eq!(r.body["buyer_pays"], "80001389");
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/auctions/{id}"),
        None,
        &w.cookies,
    )
    .await;
    assert_eq!(r.body["state"], "awaiting_payment");
    assert_eq!(r.body["my"]["winner"], true);

    // 其他人（包括卖家与出价第二高的人）拿不到
    assert_eq!(
        payment(l.cookies.clone()).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        payment(s.cookies.clone()).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(payment(Vec::new()).await.status, StatusCode::UNAUTHORIZED);
    // 付款期过了也拿不到
    sqlx::query("UPDATE auction_proofs SET pay_deadline = now() - interval '1 second' WHERE auction_id = $1")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(
        code(&payment(w.cookies.clone()).await),
        "AUCTION_UNAVAILABLE"
    );
}
