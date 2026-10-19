//! 订单接口（数据按 forum-indexer 的写法直接入库）与钱包子域名的 CORS。
//! 需要 PostgreSQL（`DATABASE_URL`）与 Valkey（`VALKEY_URL`，未设置则跳过）。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

mod common;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{CHAIN_ID, ORIGIN, WALLET_ORIGIN, build, call, user};
use forum_db::PgPool;
use tower::ServiceExt;

const ESCROW: [u8; 20] = [0x44; 20];
const STREAM: [u8; 20] = [0x5e; 20];

/// 链配置与订单：`count` 张 a 买 b 卖的一次性订单（第 1 张由报价 `offer_hash` 生成），外加一张 b 买 a 卖的分期订单。
/// 返回分期订单的 id。
async fn seed_orders(db: &PgPool, a: i64, b: i64, count: i32, offer_hash: &[u8]) -> i64 {
    sqlx::query(
        "INSERT INTO chains (chain_id, name, escrow_address, stream_address, config_address, token_address,
           token_symbol, token_decimals, start_block, safe_depth, finalized_depth)
         VALUES ($1, 'test', $2, $3, decode(repeat('c0', 20), 'hex'), decode(repeat('33', 20), 'hex'),
           'USDT', 6, 0, 240, 4800)",
    )
    .bind(i64::try_from(CHAIN_ID).unwrap())
    .bind(ESCROW.as_slice())
    .bind(STREAM.as_slice())
    .execute(db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO escrow_orders (chain_id, contract, chain_order_id, buyer, seller, buyer_id, seller_id, token,
           amount, buyer_fee, instant, offer_hash, listing_hash, state, ship_deadline, level, created_block,
           updated_block)
         SELECT $1, $2, g, decode(repeat('b1', 20), 'hex'), decode(repeat('c1', 20), 'hex'), $3, $4,
           decode(repeat('33', 20), 'hex'), 1000000000000000000000000000000 + g, 500000, false,
           CASE WHEN g = 1 THEN $5 ELSE sha256(int4send(g)) END, decode(repeat('ab', 32), 'hex'), 1,
           to_timestamp(1800000000), 'safe', g, g
         FROM generate_series(1, $6) g",
    )
    .bind(i64::try_from(CHAIN_ID).unwrap())
    .bind(ESCROW.as_slice())
    .bind(a)
    .bind(b)
    .bind(offer_hash)
    .bind(count)
    .execute(db)
    .await
    .unwrap();
    sqlx::query_scalar(
        "INSERT INTO escrow_orders (chain_id, contract, chain_order_id, mode, buyer, seller, buyer_id, seller_id,
           token, amount, instant, offer_hash, listing_hash, state, ship_deadline, interval_secs, periods,
           started_at, released_amount, disputed_by, dispute_from_bond, disputed_at, level, created_block,
           updated_block)
         VALUES ($1, $2, 1, 1, decode(repeat('c1', 20), 'hex'), decode(repeat('b1', 20), 'hex'), $3, $4,
           decode(repeat('33', 20), 'hex'), 50000000, false, decode(repeat('77', 32), 'hex'),
           decode(repeat('ab', 32), 'hex'), 2, to_timestamp(1800000000), 3600, 24, to_timestamp(1800003600),
           4120833, decode(repeat('c1', 20), 'hex'), false, to_timestamp(1800010800), 'latest', 99, 99)
         RETURNING id",
    )
    .bind(i64::try_from(CHAIN_ID).unwrap())
    .bind(STREAM.as_slice())
    .bind(b)
    .bind(a)
    .fetch_one(db)
    .await
    .unwrap()
}

async fn insert_offer(db: &PgPool, seller: i64, buyer: i64, offer_hash: &[u8]) -> i64 {
    let (conversation, _) = forum_db::messages::get_or_create(db, seller, buyer)
        .await
        .unwrap();
    sqlx::query_scalar(
        "INSERT INTO offers (conversation_id, seller_id, buyer_id, chain_id, verifying_contract, seller, buyer, token,
           amount, fee_payer, max_fee_bps, max_base_gas, max_min_fee, bond, late_slash_bps, instant, ship_window,
           confirm_window, listing_hash, expiry, max_fills, nonce, signature, offer_hash, status, expires_at)
         VALUES ($6, $1, $2, $3, $4, decode(repeat('c1', 20), 'hex'), decode(repeat('b1', 20), 'hex'),
           decode(repeat('33', 20), 'hex'), 5000000, 0, 100, 100000, 1000000, 0, 0, false, 86400, 86400,
           decode(repeat('ab', 32), 'hex'), 1800000000, 1, 1, '\\x01', $5, 'paid', now() + interval '1 day')
         RETURNING id",
    )
    .bind(seller)
    .bind(buyer)
    .bind(i64::try_from(CHAIN_ID).unwrap())
    .bind(ESCROW.as_slice())
    .bind(offer_hash)
    .bind(conversation.id)
    .fetch_one(db)
    .await
    .unwrap()
}

fn ids(body: &serde_json::Value) -> Vec<i64> {
    body["orders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["id"].as_i64().unwrap())
        .collect()
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn my_orders_page_by_role(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let (a, ca) = user(&app, &db).await;
    let (b, cb) = user(&app, &db).await;
    let stream_id = seed_orders(&db, a, b, 25, &[1; 32]).await;

    let r = call(&app, "GET", "/api/v1/orders?role=buyer", None, &[]).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    let r = call(&app, "GET", "/api/v1/orders?role=admin", None, &ca).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);

    let r = call(&app, "GET", "/api/v1/orders?role=buyer", None, &ca).await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    let first = ids(&r.body);
    assert_eq!(first.len(), 20);
    assert!(first.windows(2).all(|w| w[0] > w[1]), "newest first");
    let cursor = r.body["next_cursor"].as_i64().unwrap();
    assert_eq!(cursor, *first.last().unwrap());
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/orders?role=buyer&cursor={cursor}"),
        None,
        &ca,
    )
    .await;
    assert_eq!(ids(&r.body).len(), 5);
    assert_eq!(r.body["next_cursor"], serde_json::Value::Null);

    // a 卖出的只有那张分期订单；b 作为买家看到同一张
    let r = call(&app, "GET", "/api/v1/orders?role=seller", None, &ca).await;
    assert_eq!(ids(&r.body), vec![stream_id]);
    let r = call(&app, "GET", "/api/v1/orders?role=buyer", None, &cb).await;
    assert_eq!(ids(&r.body), vec![stream_id]);
    let o = &r.body["orders"][0];
    assert_eq!(o["mode"], "stream");
    assert_eq!(o["state"], "disputed");
    assert_eq!(o["level"], "latest");
    assert_eq!(o["deadlines"]["ship"], serde_json::Value::Null);
    assert_eq!(o["deadlines"]["start"], "2027-01-15T08:00:00Z");
    assert_eq!(o["stream"]["periods"], 24);
    assert_eq!(o["stream"]["claimed"], "4120833");
    assert_eq!(o["stream"]["ends_at"], "2027-01-16T09:00:00Z");
    assert_eq!(o["dispute"]["disputed_at"], "2027-01-15T11:00:00Z");

    // 被区块回滚撤销的订单不在列表里
    sqlx::query("UPDATE escrow_orders SET state = 0 WHERE id = $1")
        .bind(stream_id)
        .execute(&db)
        .await
        .unwrap();
    let r = call(&app, "GET", "/api/v1/orders?role=seller", None, &ca).await;
    assert!(ids(&r.body).is_empty());
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn order_detail_for_parties_and_arbiters_only(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let (a, ca) = user(&app, &db).await;
    let (b, cb) = user(&app, &db).await;
    let (c, cc) = user(&app, &db).await;
    let offer_hash = [9u8; 32];
    seed_orders(&db, a, b, 2, &offer_hash).await;
    let offer_id = insert_offer(&db, b, a, &offer_hash).await;
    let order_id: i64 =
        sqlx::query_scalar("SELECT id FROM escrow_orders WHERE mode = 0 AND chain_order_id = 1")
            .fetch_one(&db)
            .await
            .unwrap();
    let path = format!("/api/v1/orders/{order_id}");

    for cookies in [&ca, &cb] {
        let r = call(&app, "GET", &path, None, cookies).await;
        assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
        let o = &r.body;
        assert_eq!(o["mode"], "escrow");
        assert_eq!(o["state"], "funded");
        assert_eq!(o["level"], "safe");
        assert_eq!(
            o["amount"], "1000000000000000000000000000001",
            "uint256 kept as decimal"
        );
        assert_eq!(o["fees"]["buyer_fee"], "500000");
        assert_eq!(o["fees"]["seller_fee"], serde_json::Value::Null);
        assert_eq!(o["buyer"]["user_id"], a);
        assert_eq!(o["seller"]["user_id"], b);
        assert_eq!(o["contract"], format!("0x{}", "44".repeat(20)));
        assert_eq!(o["deadlines"]["ship"], "2027-01-15T08:00:00Z");
        assert_eq!(o["stream"], serde_json::Value::Null);
        assert_eq!(o["source"]["offer_id"], offer_id);
        assert_eq!(o["source"]["offer_hash"], format!("0x{}", "09".repeat(32)));
        assert_eq!(o["source"]["auction"], serde_json::Value::Null);
    }
    // 人民币拍卖成交（ADR 0011）：成交价以分计，订单金额是按卖家汇率换算后的 USDT
    let auction_order: i64 = sqlx::query_scalar(
        "UPDATE escrow_orders SET auction_id = 42, auction_currency = 1, auction_cny_per_usdt = 72000,
           auction_price = 57600, amount = 80000000
         WHERE mode = 0 AND chain_order_id = 2 RETURNING id",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/orders/{auction_order}"),
        None,
        &ca,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert_eq!(
        r.body["source"]["auction"],
        serde_json::json!({"auction_id": "42", "currency": "cny", "cny_per_usdt": 72000, "price": "57600"})
    );
    assert_eq!(r.body["amount"], "80000000");
    // 旁人看不到，与不存在一样
    let r = call(&app, "GET", &path, None, &cc).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    let r = call(&app, "GET", "/api/v1/orders/999999", None, &ca).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    // 该订单仲裁案件的仲裁员可以
    common::assign_arbitrator(&db, order_id, c).await;
    let r = call(&app, "GET", &path, None, &cc).await;
    assert_eq!(r.status, StatusCode::OK);

    // 报价卡片 → 订单：只有该订单的买卖双方
    let by_offer = format!("/api/v1/offers/{offer_id}/order");
    for cookies in [&ca, &cb] {
        let r = call(&app, "GET", &by_offer, None, cookies).await;
        assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
        assert_eq!(r.body["id"], order_id);
    }
    let r = call(&app, "GET", &by_offer, None, &cc).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND, "arbiters use the order id");
    let r = call(&app, "GET", "/api/v1/offers/999999/order", None, &ca).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

async fn preflight(app: &Router, origin: &str) -> axum::http::HeaderMap {
    let req = Request::builder()
        .method("OPTIONS")
        .uri("/api/v1/wallet/account")
        .header(header::ORIGIN, origin)
        .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
        .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "content-type")
        .body(Body::empty())
        .unwrap();
    app.clone().oneshot(req).await.unwrap().headers().clone()
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn cors_only_for_the_wallet_origin(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let h = preflight(&app, WALLET_ORIGIN).await;
    assert_eq!(h[header::ACCESS_CONTROL_ALLOW_ORIGIN], WALLET_ORIGIN);
    assert_eq!(h[header::ACCESS_CONTROL_ALLOW_CREDENTIALS], "true");
    let methods = h[header::ACCESS_CONTROL_ALLOW_METHODS].to_str().unwrap();
    for m in ["GET", "POST", "PATCH", "DELETE"] {
        assert!(methods.contains(m), "{methods}");
    }
    assert_eq!(h[header::ACCESS_CONTROL_ALLOW_HEADERS], "content-type");

    // 论坛主站与其他来源：没有任何 CORS 头
    for origin in [ORIGIN, "https://evil.test"] {
        let h = preflight(&app, origin).await;
        assert!(
            !h.keys()
                .any(|k| k.as_str().starts_with("access-control-allow")),
            "{origin}: {h:?}"
        );
    }

    // 实际请求也带上 CORS 头（浏览器据此允许钱包页读取响应）
    let req = Request::builder()
        .uri("/api/v1/trade/params")
        .header(header::ORIGIN, WALLET_ORIGIN)
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
        WALLET_ORIGIN
    );
}
