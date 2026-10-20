//! 交易帖与担保约定卡片：发帖、报价（EIP-712 哈希、范围校验）、修改 / 拒绝 / 取消 / 接受、可见性。
//! 需要 PostgreSQL（`DATABASE_URL`）与 Valkey（`VALKEY_URL`，未设置则跳过）。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

mod common;

use axum::Router;
use axum::http::StatusCode;
use common::{
    CHAIN_ID, ESCROW, USDT, age, build, call, clear_resend, enable_trading, jar, login, user,
};
use forum_core::ErrorCode;
use forum_core::eip712::{Domain, Offer};
use forum_db::PgPool;
use forum_db::forums::{self, NewForum};
use serde_json::{Value, json};

const SELLER: &str = "0x1111111111111111111111111111111111111111";
const BUYER: &str = "0x2222222222222222222222222222222222222222";

async fn market(db: &PgPool) -> i32 {
    board(db, "market").await
}

async fn board(db: &PgPool, slug: &str) -> i32 {
    forums::create(
        db,
        &NewForum {
            parent_id: None,
            slug,
            name: slug,
            description: "",
            position: 0,
            view_min_level: 0,
            post_min_level: 1,
            reply_min_level: 1,
            allow_trade: true,
            color: None,
        },
    )
    .await
    .unwrap()
}

struct Party {
    id: i64,
    cookies: Vec<String>,
}

/// 卖家与对方的私信会话。
async fn conversation(app: &Router, seller: &Party, peer: i64) -> i64 {
    let r = call(
        app,
        "POST",
        "/api/v1/conversations",
        Some(json!({"user_id": peer})),
        &seller.cookies,
    )
    .await;
    assert!(r.status.is_success(), "{:?}", r.body);
    r.body["id"].as_i64().unwrap()
}

/// 已开通交易、已解除新用户限制的用户；会话带 5 分钟近期 Passkey 验证。
async fn trader(app: &Router, db: &PgPool, address: &str) -> Party {
    let (id, cookies) = user(app, db).await;
    age(db, id).await;
    enable_trading(app, db, id, &cookies, address).await;
    Party { id, cookies }
}

fn listing_body(price: &str, ship: u32) -> Value {
    json!({
        "title": "二手键盘",
        "html": "<p>九成新</p>",
        "source": {"type": "doc"},
        "price": price,
        "terms": {"ship_window": ship, "confirm_window": 7 * 86_400}
    })
}

fn now() -> u64 {
    time::OffsetDateTime::now_utc().unix_timestamp() as u64
}

fn offer(listing_hash: &str, buyer: &str) -> Value {
    json!({
        "seller": SELLER,
        "buyer": buyer,
        "token": USDT,
        "amount": "45000000",
        "feePayer": 2,
        "maxFeeBps": 100,
        "maxBaseGas": "100000",
        "maxMinFee": "1000000",
        "bond": "0",
        "lateSlashBps": 0,
        "instant": false,
        "shipWindow": 3 * 86_400,
        "confirmWindow": 7 * 86_400,
        "listingHash": listing_hash,
        "expiry": now() + 86_400,
        "maxFills": 1,
        "nonce": "115792089237316195423570985008687907853269984665640564039457584007913129639935"
    })
}

const ZERO_ADDR: &str = "0x0000000000000000000000000000000000000000";
const ZERO_HASH: &str = "0x0000000000000000000000000000000000000000000000000000000000000000";

async fn post_offer(app: &Router, cookies: &[String], body: Value) -> common::Res {
    call(app, "POST", "/api/v1/offers", Some(body), cookies).await
}

/// 发交易帖，返回 `(listing_id, content_hash)`。
async fn listing(app: &Router, seller: &Party) -> (i64, String) {
    let r = call(
        app,
        "POST",
        "/api/v1/forums/market/listings",
        Some(listing_body("45000000", 3 * 86_400)),
        &seller.cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    (
        r.body["listing_id"].as_i64().unwrap(),
        r.body["content_hash"].as_str().unwrap().to_owned(),
    )
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn params_expose_domain_and_bounds(db: PgPool) {
    let Some(app) = build(db) else { return };
    let r = call(&app, "GET", "/api/v1/trade/params", None, &[]).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.body["domain"]["name"], "AkariEscrow");
    assert_eq!(r.body["domain"]["version"], "1");
    assert_eq!(r.body["domain"]["chainId"], CHAIN_ID);
    assert_eq!(r.body["domain"]["verifyingContract"], ESCROW);
    assert_eq!(r.body["min_order"], "5000000");
    assert_eq!(r.body["order_cap"], "200000000");
    assert_eq!(r.body["min_window"], 86_400);
    assert_eq!(r.body["max_window"], 30 * 86_400);
    assert_eq!(r.body["defaults"]["private_offer_ttl"], 86_400);
    let versions = r.body["wallet_init_versions"].as_array().unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0]["version"], 1);
    assert_eq!(
        versions[0]["kernel_factory"],
        "0x2577507b78c2008ff367261cb6285d44ba5ef2e9"
    );
    assert_eq!(
        versions[0]["approve_amount"],
        alloy::primitives::U256::MAX.to_string()
    );
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn listing_only_on_trade_boards(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    forums::create(
        &db,
        &NewForum {
            parent_id: None,
            slug: "chat",
            name: "chat",
            description: "",
            position: 0,
            view_min_level: 0,
            post_min_level: 1,
            reply_min_level: 1,
            allow_trade: false,
            color: None,
        },
    )
    .await
    .unwrap();
    let seller = trader(&app, &db, SELLER).await;
    let r = call(
        &app,
        "POST",
        "/api/v1/forums/chat/listings",
        Some(listing_body("45000000", 86_400)),
        &seller.cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert_eq!(r.body["code"], "TRADE_NOT_ALLOWED");
    let topics: i64 = sqlx::query_scalar("SELECT count(*) FROM topics")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(topics, 0);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn listing_requires_wallet_and_valid_terms(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    market(&db).await;
    let (id, plain) = user(&app, &db).await;
    age(&db, id).await;
    let path = "/api/v1/forums/market/listings";
    let r = call(
        &app,
        "POST",
        path,
        Some(listing_body("45000000", 86_400)),
        &plain,
    )
    .await;
    assert_eq!(r.body["code"], "WALLET_REQUIRED");

    let seller = trader(&app, &db, SELLER).await;
    for (body, code) in [
        (listing_body("4999999", 86_400), "AMOUNT_OUT_OF_RANGE"),
        (listing_body("200000001", 86_400), "AMOUNT_OUT_OF_RANGE"),
        (listing_body("45000000", 86_399), "WINDOW_OUT_OF_RANGE"),
        (
            listing_body("45000000", 30 * 86_400 + 1),
            "WINDOW_OUT_OF_RANGE",
        ),
        (listing_body("4.5", 86_400), "INVALID_PARAMETER"),
    ] {
        let r = call(&app, "POST", path, Some(body), &seller.cookies).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST);
        assert_eq!(r.body["code"], code);
    }
    // 参考汇率超出 5.0000–10.0000（如把 7.2 填成 72 或 0.72）
    for rate in [72, 7_200, 49_999, 100_001] {
        let mut body = listing_body("45000000", 86_400);
        body["cny_per_usdt"] = json!(rate);
        let r = call(&app, "POST", path, Some(body), &seller.cookies).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST);
        assert_eq!(r.body["code"], "INVALID_PARAMETER", "{rate}");
    }
    let mut bond = listing_body("45000000", 86_400);
    bond["terms"]["bond"] = json!("45000001");
    let r = call(&app, "POST", path, Some(bond), &seller.cookies).await;
    assert_eq!(r.body["code"], "BOND_INVALID");

    let (listing_id, hash) = listing(&app, &seller).await;
    assert_eq!(hash.len(), 66);
    let (kind, topic_id): (i16, i64) = sqlx::query_as(
        "SELECT t.kind, t.id FROM topics t JOIN trade_listings l ON l.topic_id = t.id WHERE l.id = $1",
    )
    .bind(listing_id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(kind, 1, "trade topic");

    let r = call(
        &app,
        "GET",
        &format!("/api/v1/listings/{listing_id}"),
        None,
        &[],
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.body["topic_id"], topic_id);
    assert_eq!(r.body["price"], "45000000");
    assert_eq!(r.body["content_hash"], hash);
    assert_eq!(r.body["terms"]["bond"], "0");
    assert_eq!(r.body["cny_per_usdt"], Value::Null);
    // 主题帖本身照常可读；主题、板块列表、跨板块最新都带交易帖信息行（交易帖 ID 与价格）
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/topics/{topic_id}"),
        None,
        &[],
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.body["topic"]["trade"]["listing_id"], listing_id);
    assert_eq!(r.body["topic"]["trade"]["price"], "45000000");
    assert_eq!(r.body["forum"]["allow_trade"], true);
    for path in ["/api/v1/forums/market/topics", "/api/v1/topics/latest"] {
        let r = call(&app, "GET", path, None, &[]).await;
        assert_eq!(r.status, StatusCode::OK, "{path}");
        let t = &r.body["topics"][0];
        assert_eq!(t["id"], topic_id, "{path}");
        assert_eq!(t["trade"]["listing_id"], listing_id, "{path}");
        assert_eq!(t["trade"]["price"], "45000000", "{path}");
        // 还没有公开报价：没有库存数字；拍卖位留空
        assert_eq!(t["trade"]["stock"], Value::Null, "{path}");
        assert_eq!(t["auction"], Value::Null, "{path}");
    }
    // 隐藏后交易帖也看不到
    sqlx::query("UPDATE topics SET flags = 8 WHERE id = $1")
        .bind(topic_id)
        .execute(&db)
        .await
        .unwrap();
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/listings/{listing_id}"),
        None,
        &[],
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);

    // 参考汇率只供显示：照存照返，不进内容哈希
    let mut with_rate = listing_body("45000000", 3 * 86_400);
    with_rate["cny_per_usdt"] = json!(72_000);
    let r = call(&app, "POST", path, Some(with_rate), &seller.cookies).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    assert_eq!(r.body["content_hash"], hash);
    let rated = r.body["listing_id"].as_i64().unwrap();
    let r = call(&app, "GET", &format!("/api/v1/listings/{rated}"), None, &[]).await;
    assert_eq!(r.body["cny_per_usdt"], 72_000);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn public_offer_hash_and_validation(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    market(&db).await;
    let seller = trader(&app, &db, SELLER).await;
    let buyer = trader(&app, &db, BUYER).await;
    let (listing_id, hash) = listing(&app, &seller).await;
    let body = |o: Value| json!({"listing_id": listing_id, "offer": o, "signature": "0x1234"});

    // 条款超出合约范围：签名前就失败
    let mut o = offer(&hash, ZERO_ADDR);
    o["maxFeeBps"] = json!(99);
    assert_eq!(
        post_offer(&app, &seller.cookies, body(o)).await.body["code"],
        "FEE_LIMIT_INVALID"
    );
    let mut o = offer(&hash, ZERO_ADDR);
    o["confirmWindow"] = json!(3600);
    assert_eq!(
        post_offer(&app, &seller.cookies, body(o)).await.body["code"],
        "WINDOW_OUT_OF_RANGE"
    );
    let mut o = offer(&hash, ZERO_ADDR);
    o["expiry"] = json!(now() + 31 * 86_400);
    assert_eq!(
        post_offer(&app, &seller.cookies, body(o)).await.body["code"],
        "OFFER_EXPIRY_INVALID"
    );
    let mut o = offer(&hash, ZERO_ADDR);
    o["token"] = json!(BUYER);
    assert_eq!(
        post_offer(&app, &seller.cookies, body(o)).await.body["code"],
        "INVALID_PARAMETER"
    );
    // listingHash 与交易帖不符
    assert_eq!(
        post_offer(&app, &seller.cookies, body(offer(ZERO_HASH, ZERO_ADDR)))
            .await
            .body["code"],
        "INVALID_PARAMETER"
    );
    // 报价里的卖家地址不是自己的钱包；别人的交易帖
    let mut o = offer(&hash, ZERO_ADDR);
    o["seller"] = json!(BUYER);
    assert_eq!(
        post_offer(&app, &buyer.cookies, body(o.clone()))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post_offer(&app, &seller.cookies, body(o)).await.status,
        StatusCode::FORBIDDEN
    );
    // 未知字段（防止前端签的结构与服务端理解的不一致）
    let mut o = offer(&hash, ZERO_ADDR);
    o["mode"] = json!(0);
    assert_eq!(
        post_offer(&app, &seller.cookies, body(o)).await.status,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    let o = offer(&hash, ZERO_ADDR);
    let r = post_offer(&app, &seller.cookies, body(o.clone())).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    // 服务端重算的哈希 = forum_core 的 hashOffer（其编码已与 Solidity 向量交叉校验）
    let typed: Offer = serde_json::from_value(o.clone()).unwrap();
    let domain = Domain {
        chain_id: CHAIN_ID,
        verifying_contract: ESCROW.parse().unwrap(),
    };
    assert_eq!(r.body["offer_hash"], typed.hash(&domain).to_string());
    assert_eq!(r.body["offer"], o, "typed data round-trips unchanged");
    assert_eq!(r.body["status"], "pending");
    assert_eq!(r.body["buyer_id"], Value::Null);
    // 同一份报价不能提交两次
    assert_eq!(
        post_offer(&app, &seller.cookies, body(o)).await.status,
        StatusCode::CONFLICT
    );

    // 公开报价：游客也能在交易帖下看到
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/listings/{listing_id}/offers"),
        None,
        &[],
    )
    .await;
    assert_eq!(r.body.as_array().unwrap().len(), 1);
    let offer_id = r.body[0]["id"].as_i64().unwrap();

    // 买家接受：拿到付款数据。各半：手续费 max(0.45, 1) = 1 USDT、gas 0.1 USDT，买家各付一半
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/offers/{offer_id}/accept"),
        None,
        &buyer.cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert_eq!(r.body["escrow"], ESCROW);
    assert_eq!(r.body["buyer_pays"], "45550000");
    assert_eq!(r.body["buyer_fee"], "500000");
    assert_eq!(r.body["buyer_gas"], "50000");
    assert_eq!(r.body["buyer_wallet"], BUYER);
    assert_eq!(r.body["signature"], "0x1234");
    // 卖家不能接受自己的报价；公开报价不能拒绝
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/offers/{offer_id}/accept"),
        None,
        &seller.cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/offers/{offer_id}/reject"),
        Some(json!({})),
        &buyer.cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    // 未开通交易的用户不能接受
    let (_, plain) = user(&app, &db).await;
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/offers/{offer_id}/accept"),
        None,
        &plain,
    )
    .await;
    assert_eq!(r.body["code"], "WALLET_REQUIRED");

    // 过期：读取时立即按已过期展示，接受被拒；worker 落库前后结果一致
    sqlx::query("UPDATE offers SET expires_at = now() - interval '1 second' WHERE id = $1")
        .bind(offer_id)
        .execute(&db)
        .await
        .unwrap();
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/offers/{offer_id}"),
        None,
        &[],
    )
    .await;
    assert_eq!(r.body["status"], "expired");
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/offers/{offer_id}/accept"),
        None,
        &buyer.cookies,
    )
    .await;
    assert_eq!(r.body["code"], "OFFER_UNAVAILABLE");
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/offers/{offer_id}/cancel"),
        None,
        &seller.cookies,
    )
    .await;
    assert_eq!(r.body["code"], "OFFER_UNAVAILABLE");

    // 公开报价跟随交易帖可见性：主题被隐藏后游客看不到，卖家本人仍可看
    sqlx::query("UPDATE topics SET flags = 8 FROM trade_listings l WHERE l.id = $1 AND topics.id = l.topic_id")
        .bind(listing_id)
        .execute(&db)
        .await
        .unwrap();
    let one = format!("/api/v1/offers/{offer_id}");
    assert_eq!(
        call(&app, "GET", &one, None, &[]).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&app, "GET", &one, None, &seller.cookies).await.status,
        StatusCode::OK
    );
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn offer_needs_recent_passkey(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let seller = trader(&app, &db, SELLER).await;
    let buyer = trader(&app, &db, BUYER).await;
    let conv = conversation(&app, &seller, buyer.id).await;
    let body =
        json!({"conversation_id": conv, "offer": offer(ZERO_HASH, BUYER), "signature": "0x12"});
    // 另一台设备上的新会话：只凭邮箱登录发不出担保报价
    let email: String = sqlx::query_scalar("SELECT email FROM users WHERE id = $1")
        .bind(seller.id)
        .fetch_one(&db)
        .await
        .unwrap();
    clear_resend(&email).await;
    let fresh = jar(&login(&app, &db, &email, &[]).await.cookies);
    let r = post_offer(&app, &fresh, body.clone()).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert_eq!(r.body["code"], "PASSKEY_REQUIRED");
    assert_eq!(
        post_offer(&app, &seller.cookies, body).await.status,
        StatusCode::CREATED
    );
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn conversation_cards_modify_reject_cancel(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let seller = trader(&app, &db, SELLER).await;
    let buyer = trader(&app, &db, BUYER).await;
    let (stranger_id, stranger) = user(&app, &db).await;
    let conv = conversation(&app, &seller, buyer.id).await;
    let body = |o: Value, replaces: Option<i64>| json!({"conversation_id": conv, "replaces": replaces, "offer": o, "signature": "0xabcd"});

    // 私信报价必须指定买家且只能成交 1 次；买家必须已开通交易
    assert_eq!(
        post_offer(
            &app,
            &seller.cookies,
            body(offer(ZERO_HASH, ZERO_ADDR), None)
        )
        .await
        .body["code"],
        "INVALID_PARAMETER"
    );
    let mut o = offer(ZERO_HASH, BUYER);
    o["maxFills"] = json!(2);
    assert_eq!(
        post_offer(&app, &seller.cookies, body(o, None)).await.body["code"],
        "INVALID_PARAMETER"
    );
    let unknown = offer(ZERO_HASH, "0x9999999999999999999999999999999999999999");
    assert_eq!(
        post_offer(&app, &seller.cookies, body(unknown, None))
            .await
            .body["code"],
        "INVALID_PARAMETER"
    );

    let r = post_offer(&app, &seller.cookies, body(offer(ZERO_HASH, BUYER), None)).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    let first = r.body["id"].as_i64().unwrap();
    assert_eq!(r.body["buyer_id"], buyer.id);

    // 只有买卖双方看得到
    let list = format!("/api/v1/conversations/{conv}/offers");
    assert_eq!(
        call(&app, "GET", &list, None, &buyer.cookies)
            .await
            .body
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        call(&app, "GET", &list, None, &stranger)
            .await
            .body
            .as_array()
            .unwrap()
            .is_empty()
    );
    let one = format!("/api/v1/offers/{first}");
    assert_eq!(
        call(&app, "GET", &one, None, &stranger).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&app, "GET", &one, None, &[]).await.status,
        StatusCode::NOT_FOUND
    );
    // 别人不能接受指定给买家的报价
    let r = call(&app, "POST", &format!("{one}/accept"), None, &stranger).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);

    // 修改约定：新卡片替换旧卡片，旧卡片变为已修改
    let mut changed = offer(ZERO_HASH, BUYER);
    changed["amount"] = json!("40000000");
    let r = post_offer(&app, &seller.cookies, body(changed.clone(), Some(first))).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    let second = r.body["id"].as_i64().unwrap();
    assert_eq!(r.body["replaces_id"], first);
    assert_eq!(
        call(&app, "GET", &one, None, &seller.cookies).await.body["status"],
        "modified"
    );
    // 已作废的卡片不能再被修改、接受
    let mut again = changed.clone();
    again["nonce"] = json!("2");
    let r = post_offer(&app, &seller.cookies, body(again, Some(first))).await;
    assert_eq!(r.body["code"], "OFFER_UNAVAILABLE");
    let r = call(&app, "POST", &format!("{one}/accept"), None, &buyer.cookies).await;
    assert_eq!(r.body["code"], "OFFER_UNAVAILABLE");
    // 买家不能替换卖家的卡片
    let r = post_offer(
        &app,
        &buyer.cookies,
        body(offer(ZERO_HASH, SELLER), Some(second)),
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);

    // 拒绝：只有指定买家能拒绝，可附理由
    let reject = format!("/api/v1/offers/{second}/reject");
    let r = call(
        &app,
        "POST",
        &reject,
        Some(json!({"reason": "太贵"})),
        &seller.cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let long = "长".repeat(201);
    let r = call(
        &app,
        "POST",
        &reject,
        Some(json!({"reason": long})),
        &buyer.cookies,
    )
    .await;
    assert_eq!(r.body["code"], "INVALID_PARAMETER");
    let r = call(
        &app,
        "POST",
        &reject,
        Some(json!({"reason": " 太贵 "})),
        &buyer.cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert_eq!(r.body["status"], "rejected");
    assert_eq!(r.body["reject_reason"], "太贵");
    // 已拒绝的不能再取消
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/offers/{second}/cancel"),
        None,
        &seller.cookies,
    )
    .await;
    assert_eq!(r.body["code"], "OFFER_UNAVAILABLE");

    // 取消：只有卖家能取消
    let mut third = changed;
    third["nonce"] = json!("3");
    let id3 = post_offer(&app, &seller.cookies, body(third, None))
        .await
        .body["id"]
        .as_i64()
        .unwrap();
    let cancel = format!("/api/v1/offers/{id3}/cancel");
    assert_eq!(
        call(&app, "POST", &cancel, None, &buyer.cookies)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    let r = call(&app, "POST", &cancel, None, &seller.cookies).await;
    assert_eq!(r.body["status"], "cancelled");

    let r = call(&app, "GET", &list, None, &seller.cookies).await;
    let statuses: Vec<&str> = r
        .body
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["status"].as_str().unwrap())
        .collect();
    assert_eq!(statuses, ["cancelled", "rejected", "modified"]);

    // 每次状态变化都在会话里留一张卡片消息，并通知对方
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/conversations/{conv}/messages"),
        None,
        &buyer.cookies,
    )
    .await;
    let cards: Vec<(String, i64, i64)> = r.body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            assert_eq!(m["kind"], "offer");
            (
                m["body"].as_str().unwrap().to_owned(),
                m["offer_id"].as_i64().unwrap(),
                m["sender_id"].as_i64().unwrap(),
            )
        })
        .collect();
    let (s, b) = (seller.id, buyer.id);
    assert_eq!(
        cards,
        [
            ("created".into(), first, s),
            ("modified".into(), second, s),
            ("rejected".into(), second, b),
            ("created".into(), id3, s),
            ("cancelled".into(), id3, s),
        ]
    );
    let notified: Vec<i64> = sqlx::query_scalar(
        "SELECT (payload->>'to')::bigint FROM outbox WHERE kind = 'notify.offer' ORDER BY id",
    )
    .fetch_all(&db)
    .await
    .unwrap();
    assert_eq!(notified, [b, b, s, b, b]);

    // 报价只能发在卖家与指定买家两人的会话里
    let other = conversation(&app, &seller, stranger_id).await;
    let mut o = offer(ZERO_HASH, BUYER);
    o["nonce"] = json!("9");
    let r = post_offer(
        &app,
        &seller.cookies,
        json!({"conversation_id": other, "offer": o, "signature": "0xabcd"}),
    )
    .await;
    assert_eq!(r.body["code"], "INVALID_PARAMETER");
    let r = post_offer(
        &app,
        &seller.cookies,
        json!({"conversation_id": 999_999, "offer": o, "signature": "0xabcd"}),
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

/// 有效的封号 / 禁拍（`by` 是任一用户，只用于记录处罚人）。
async fn sanction(db: &PgPool, kind: i16, user_id: i64, forum_id: Option<i32>, by: i64) -> i64 {
    forum_db::sanctions::create(
        db,
        forum_db::sanctions::NewSanction {
            kind,
            user_id,
            forum_id,
            ends_at: None,
            reason: "test",
            created_by: Some(by),
        },
    )
    .await
    .unwrap()
}

/// buyer 买、seller 卖的已付款订单（按 forum-indexer 的写法直接入库）。
async fn paid_order(db: &PgPool, buyer: i64, seller: i64) -> i64 {
    sqlx::query(
        "INSERT INTO chains (chain_id, name, escrow_address, stream_address, config_address, token_address,
           token_symbol, token_decimals, start_block, safe_depth, finalized_depth)
         VALUES ($1, 'test', decode(repeat('44', 20), 'hex'), decode(repeat('5e', 20), 'hex'),
           decode(repeat('c0', 20), 'hex'), decode(repeat('33', 20), 'hex'), 'USDT', 6, 0, 240, 4800)",
    )
    .bind(i64::try_from(CHAIN_ID).unwrap())
    .execute(db)
    .await
    .unwrap();
    sqlx::query_scalar(
        "INSERT INTO escrow_orders (chain_id, contract, chain_order_id, buyer, seller, buyer_id, seller_id, token,
           amount, buyer_fee, instant, offer_hash, listing_hash, state, ship_deadline, level, created_block,
           updated_block)
         VALUES ($1, decode(repeat('44', 20), 'hex'), 1, decode(repeat('b1', 20), 'hex'),
           decode(repeat('c1', 20), 'hex'), $2, $3, decode(repeat('33', 20), 'hex'), 45000000, 500000, false,
           decode(repeat('01', 32), 'hex'), decode(repeat('ab', 32), 'hex'), 1, to_timestamp(1800000000),
           'safe', 1, 1)
         RETURNING id",
    )
    .bind(i64::try_from(CHAIN_ID).unwrap())
    .bind(buyer)
    .bind(seller)
    .fetch_one(db)
    .await
    .unwrap()
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn banned_users_cannot_start_trades_but_keep_their_orders(db: PgPool) {
    use forum_db::sanctions::{KIND_AUCTION_BAN, KIND_BAN};

    let Some(state) = common::state_with(db.clone(), common::wallet_state(), &common::namespace())
    else {
        return;
    };
    let app = common::router_of(state.clone());
    let market_id = market(&db).await;
    let other = board(&db, "other").await;
    let seller = trader(&app, &db, SELLER).await;
    let buyer = trader(&app, &db, BUYER).await;
    let (listing_id, hash) = listing(&app, &seller).await;
    let body = |nonce: &str| {
        let mut o = offer(&hash, ZERO_ADDR);
        o["nonce"] = json!(nonce);
        json!({"listing_id": listing_id, "offer": o, "signature": "0x1234"})
    };
    let r = post_offer(&app, &seller.cookies, body("1")).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    let open = r.body["id"].as_i64().unwrap();
    let order = paid_order(&db, buyer.id, seller.id).await;

    // 卖家被封号：不能发交易帖、不能提交或修改报价卡片
    let ban = sanction(&db, KIND_BAN, seller.id, None, buyer.id).await;
    let r = call(
        &app,
        "POST",
        "/api/v1/forums/market/listings",
        Some(listing_body("45000000", 3 * 86_400)),
        &seller.cookies,
    )
    .await;
    assert_eq!(r.body["code"], "ACCOUNT_BANNED");
    assert_eq!(
        post_offer(&app, &seller.cookies, body("2")).await.body["code"],
        "ACCOUNT_BANNED"
    );
    let mut replace = body("3");
    replace["replaces"] = json!(open);
    assert_eq!(
        post_offer(&app, &seller.cookies, replace).await.body["code"],
        "ACCOUNT_BANNED"
    );
    // 已有订单照常：详情、订单聊天、提币地址
    let path = format!("/api/v1/orders/{order}");
    assert_eq!(
        call(&app, "GET", &path, None, &seller.cookies).await.status,
        StatusCode::OK
    );
    let r = call(
        &app,
        "POST",
        &format!("{path}/messages"),
        Some(json!({"body": "已发货"})),
        &seller.cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    let r = common::wallet_call(
        &app,
        "GET",
        "/api/v1/wallet/withdraw-addresses",
        None,
        &seller.cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    sqlx::query("UPDATE sanctions SET revoked_at = now(), revoked_by = $2 WHERE id = $1")
        .bind(ban)
        .bind(buyer.id)
        .execute(&db)
        .await
        .unwrap();

    // 买家被封号：不能接受报价（开始新订单）
    sanction(&db, KIND_BAN, buyer.id, None, seller.id).await;
    let accept = format!("/api/v1/offers/{open}/accept");
    let r = call(&app, "POST", &accept, None, &buyer.cookies).await;
    assert_eq!(r.body["code"], "ACCOUNT_BANNED");

    // 拍卖入口的检查：封号优先；禁拍只管所在板块与全站
    let ab = |id| forum_api::posting::check_can_auction(&state, id, Some(market_id));
    let code = |r: Result<(), forum_api::error::ApiError>| r.err().map(|e| e.0);
    assert_eq!(code(ab(buyer.id).await), Some(ErrorCode::AccountBanned));
    assert_eq!(code(ab(seller.id).await), None);
    sanction(&db, KIND_AUCTION_BAN, seller.id, Some(other), buyer.id).await;
    assert_eq!(code(ab(seller.id).await), None, "other board");
    sanction(&db, KIND_AUCTION_BAN, seller.id, None, buyer.id).await;
    assert_eq!(code(ab(seller.id).await), Some(ErrorCode::AuctionBanned));
}
