//! 私信、订单聊天（哈希链）、站内通知、在线状态与 WebSocket 推送（两个实例共用 Valkey）。
//! 需要 PostgreSQL（`DATABASE_URL`）与 Valkey（`VALKEY_URL`，未设置则跳过）。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

mod common;

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use axum::Router;
use axum::http::StatusCode;
use common::{
    CHAIN_ID, ORIGIN, build, call, namespace, router_of, serve, state_with, user, wallet_state,
};
use forum_db::PgPool;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::{self, Message};

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// 一笔 `buyer` 买 `seller` 卖的进行中订单，返回订单 ID。
async fn seed_order(db: &PgPool, buyer: i64, seller: i64) -> i64 {
    sqlx::query(
        "INSERT INTO chains (chain_id, name, escrow_address, config_address, token_address, token_symbol,
           token_decimals, start_block, safe_depth, finalized_depth)
         VALUES ($1, 'test', decode(repeat('44', 20), 'hex'), decode(repeat('c0', 20), 'hex'),
           decode(repeat('33', 20), 'hex'), 'USDT', 6, 0, 240, 4800)
         ON CONFLICT DO NOTHING",
    )
    .bind(i64::try_from(CHAIN_ID).unwrap())
    .execute(db)
    .await
    .unwrap();
    sqlx::query_scalar(
        "INSERT INTO escrow_orders (chain_id, contract, chain_order_id, buyer, seller, buyer_id, seller_id, token,
           amount, instant, offer_hash, listing_hash, state, level, created_block, updated_block)
         VALUES ($1, decode(repeat('44', 20), 'hex'), (SELECT count(*) + 1 FROM escrow_orders),
           decode(repeat('b1', 20), 'hex'), decode(repeat('c1', 20), 'hex'), $2, $3, decode(repeat('33', 20), 'hex'),
           5000000, false, sha256(int8send($2 * 1000 + $3)), decode(repeat('ab', 32), 'hex'), 1, 'latest', 1, 1)
         RETURNING id",
    )
    .bind(i64::try_from(CHAIN_ID).unwrap())
    .bind(buyer)
    .bind(seller)
    .fetch_one(db)
    .await
    .unwrap()
}

async fn open_conversation(app: &Router, cookies: &[String], peer: i64) -> i64 {
    let r = call(
        app,
        "POST",
        "/api/v1/conversations",
        Some(json!({"user_id": peer})),
        cookies,
    )
    .await;
    assert!(r.status.is_success(), "{:?}", r.body);
    r.body["id"].as_i64().unwrap()
}

async fn say(app: &Router, cookies: &[String], conv: i64, body: &str) -> Value {
    let r = call(
        app,
        "POST",
        &format!("/api/v1/conversations/{conv}/messages"),
        Some(json!({"body": body})),
        cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    r.body
}

fn bodies(page: &Value) -> Vec<String> {
    page["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["body"].as_str().unwrap().to_owned())
        .collect()
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn conversations_messages_reads_and_blocks(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let (a, ca) = user(&app, &db).await;
    let (b, cb) = user(&app, &db).await;
    let (_, cs) = user(&app, &db).await;

    let open = |cookies: Vec<String>, peer: i64| {
        let app = app.clone();
        async move {
            call(
                &app,
                "POST",
                "/api/v1/conversations",
                Some(json!({"user_id": peer})),
                &cookies,
            )
            .await
        }
    };
    assert_eq!(open(ca.clone(), a).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        open(ca.clone(), 999_999).await.status,
        StatusCode::NOT_FOUND
    );
    let r = open(ca.clone(), b).await;
    assert_eq!(r.status, StatusCode::CREATED);
    let conv = r.body["id"].as_i64().unwrap();
    assert_eq!(r.body["peer"]["id"], b);
    assert_eq!(r.body["last_message"], Value::Null);
    // 同一对人只有一个会话，谁打开都一样
    let r = open(cb.clone(), a).await;
    assert_eq!(
        (r.status, r.body["id"].as_i64()),
        (StatusCode::OK, Some(conv))
    );
    // 空会话不进列表
    let r = call(&app, "GET", "/api/v1/conversations", None, &ca).await;
    assert!(r.body["conversations"].as_array().unwrap().is_empty());

    // 发消息：去首尾空白、非空、≤ 2000 字；含地址或「私下转账」的标记提醒
    let path = format!("/api/v1/conversations/{conv}/messages");
    for (body, code) in [
        ("   ", "CONTENT_EMPTY"),
        (&*"字".repeat(2001), "CONTENT_TOO_LONG"),
    ] {
        let r = call(&app, "POST", &path, Some(json!({"body": body})), &ca).await;
        assert_eq!(r.body["code"], code);
    }
    let m1 = say(&app, &ca, conv, " 你好 ").await;
    assert_eq!(
        (m1["body"].as_str(), m1["warn"].as_bool()),
        (Some("你好"), Some(false))
    );
    let addr = format!("0x{}", "ab".repeat(20));
    let m2 = say(&app, &ca, conv, &format!("私下转账到 {addr}")).await;
    assert_eq!(m2["warn"], true);
    // 旁人看不到也发不了
    assert_eq!(
        call(&app, "GET", &path, None, &cs).await.status,
        StatusCode::NOT_FOUND
    );
    let r = call(&app, "POST", &path, Some(json!({"body": "x"})), &cs).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert_eq!(
        call(
            &app,
            "GET",
            &format!("/api/v1/conversations/{conv}"),
            None,
            &cs
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );

    // 对方此前已读完才通知：连发两条只通知一次
    let notified = || async {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM outbox WHERE kind = 'notify.message'")
            .fetch_one(&db)
            .await
            .unwrap()
    };
    assert_eq!(notified().await, 1);

    let r = call(&app, "GET", "/api/v1/conversations", None, &cb).await;
    let item = &r.body["conversations"][0];
    assert_eq!(item["id"], conv);
    assert_eq!(item["unread"], 2);
    assert_eq!(item["last_message"]["id"], m2["id"]);
    assert_eq!(item["peer"]["id"], a);
    // 已读：不能超过最后一条、不后退；我自己发的直接算已读
    let read = format!("/api/v1/conversations/{conv}/read");
    let r = call(&app, "POST", &read, Some(json!({"last_id": i64::MAX})), &cb).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    let r = call(&app, "POST", &read, Some(json!({"last_id": 1})), &cb).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/conversations/{conv}"),
        None,
        &cb,
    )
    .await;
    assert_eq!(
        (r.body["unread"].as_i64(), r.body["my_read_id"].clone()),
        (Some(0), m2["id"].clone())
    );
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/conversations/{conv}"),
        None,
        &ca,
    )
    .await;
    assert_eq!(r.body["peer_read_id"], m2["id"]);
    say(&app, &ca, conv, "在吗").await;
    assert_eq!(notified().await, 2, "new unread streak");

    // 翻页：before / after，按时间正序
    for i in 0..60 {
        say(&app, &cb, conv, &format!("m{i}")).await;
    }
    let r = call(&app, "GET", &path, None, &ca).await;
    let latest = bodies(&r.body);
    assert_eq!(latest.len(), 50);
    assert_eq!(latest.last().unwrap(), "m59");
    assert_eq!(r.body["has_more"], true);
    let first_id = r.body["messages"][0]["id"].as_i64().unwrap();
    let r = call(&app, "GET", &format!("{path}?before={first_id}"), None, &ca).await;
    let older = bodies(&r.body);
    assert_eq!(older.len(), 13);
    assert_eq!(older[0], "你好");
    assert_eq!(r.body["has_more"], false);
    let r = call(
        &app,
        "GET",
        &format!("{path}?after={}", m2["id"]),
        None,
        &ca,
    )
    .await;
    assert_eq!(bodies(&r.body)[0], "在吗");
    assert_eq!(r.body["has_more"], true);
    let r = call(&app, "GET", &format!("{path}?before=1&after=1"), None, &ca).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);

    // 屏蔽：任一方屏蔽后双方都发不了；解除后恢复
    let block = format!("/api/v1/users/{a}/block");
    assert_eq!(
        call(&app, "POST", &block, None, &cb).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&app, "POST", &block, None, &cb).await.status,
        StatusCode::NO_CONTENT
    );
    for cookies in [&ca, &cb] {
        let r = call(&app, "POST", &path, Some(json!({"body": "x"})), cookies).await;
        assert_eq!(
            (r.status, r.body["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("USER_BLOCKED"))
        );
    }
    assert_eq!(open(ca.clone(), b).await.body["code"], "USER_BLOCKED");
    let r = call(&app, "GET", "/api/v1/conversations", None, &cb).await;
    assert_eq!(r.body["conversations"][0]["blocked"], true);
    assert_eq!(
        call(&app, "DELETE", &block, None, &cb).await.status,
        StatusCode::NO_CONTENT
    );
    say(&app, &ca, conv, "又能发了").await;
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn conversation_list_pages_by_last_message(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let (_, ca) = user(&app, &db).await;
    let mut convs = Vec::new();
    for i in 0..22 {
        // 对方不用登录（登录接口按 IP 限流）
        let (p, _) = forum_db::users::upsert_by_email(&db, &format!("peer{i}@example.com"))
            .await
            .unwrap();
        let p = p.id;
        let conv = open_conversation(&app, &ca, p).await;
        say(&app, &ca, conv, "hi").await;
        convs.push(conv);
    }
    // 最早的会话来了新消息，排到最前
    say(&app, &ca, convs[0], "again").await;
    let r = call(&app, "GET", "/api/v1/conversations", None, &ca).await;
    let page: Vec<i64> = r.body["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_i64().unwrap())
        .collect();
    assert_eq!(page.len(), 20);
    assert_eq!(page[0], convs[0]);
    let cursor = r.body["next_cursor"].as_i64().unwrap();
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/conversations?cursor={cursor}"),
        None,
        &ca,
    )
    .await;
    let rest: Vec<i64> = r.body["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_i64().unwrap())
        .collect();
    assert_eq!(rest, [convs[2], convs[1]]);
    assert_eq!(r.body["next_cursor"], Value::Null);
}

/// 绕过只追加触发器改库（模拟服务器被攻破后直接改数据）。
async fn tamper(db: &PgPool, sql: &'static str, id: i64) {
    let mut tx = db.begin().await.unwrap();
    sqlx::query("ALTER TABLE order_messages DISABLE TRIGGER order_messages_no_update_delete")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(sql).bind(id).execute(&mut *tx).await.unwrap();
    sqlx::query("ALTER TABLE order_messages ENABLE TRIGGER order_messages_no_update_delete")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn order_chat_hash_chain_and_access(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let (a, ca) = user(&app, &db).await;
    let (b, cb) = user(&app, &db).await;
    let (c, cc) = user(&app, &db).await;
    let order = seed_order(&db, a, b).await;
    let path = format!("/api/v1/orders/{order}/messages");

    // 旁人 404；仲裁员可读可发
    assert_eq!(
        call(&app, "GET", &path, None, &cc).await.status,
        StatusCode::NOT_FOUND
    );
    let r = call(&app, "POST", &path, Some(json!({"body": "x"})), &cc).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    forum_db::staff::grant(&db, c, forum_db::staff::ROLE_ADMIN, None, None)
        .await
        .unwrap();

    let mut ids = Vec::new();
    for (cookies, body) in [
        (&ca, "已付款"),
        (&cb, "明天发货"),
        (&cc, "请双方提供证据"),
        (&ca, "线下给你现金"),
    ] {
        let r = call(&app, "POST", &path, Some(json!({"body": body})), cookies).await;
        assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
        ids.push(r.body["id"].as_i64().unwrap());
        assert_eq!(r.body["warn"], body.contains("线下"));
    }
    let r = call(&app, "GET", &path, None, &cb).await;
    let msgs = r.body["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 4);
    assert_eq!(msgs[0]["prev_hash"], format!("0x{}", "00".repeat(32)));
    for w in msgs.windows(2) {
        assert_eq!(w[1]["prev_hash"], w[0]["hash"], "linked");
    }
    assert_eq!(msgs[2]["sender_id"], c);

    // 已读
    let read = format!("{path}/read");
    let r = call(&app, "POST", &read, Some(json!({"last_id": ids[1]})), &cb).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    let r = call(&app, "GET", &path, None, &ca).await;
    assert_eq!(r.body["reads"], json!([{"user_id": b, "last_id": ids[1]}]));

    let verify = format!("{path}/verify");
    let r = call(&app, "GET", &verify, None, &cc).await;
    assert_eq!(r.body["ok"], true, "{:?}", r.body);
    assert_eq!(r.body["count"], 4);
    assert_eq!(r.body["head_hash"], msgs[3]["hash"]);

    // 只追加：改、删都被触发器拒绝
    for sql in [
        "UPDATE order_messages SET body = 'x' WHERE id = $1",
        "DELETE FROM order_messages WHERE id = $1",
    ] {
        let err = sqlx::query(sql)
            .bind(ids[1])
            .execute(&db)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("append-only"), "{err}");
    }
    // 篡改内容：校验指出第一条被改的消息
    tamper(
        &db,
        "UPDATE order_messages SET body = '今天发货' WHERE id = $1",
        ids[1],
    )
    .await;
    let r = call(&app, "GET", &verify, None, &ca).await;
    assert_eq!(
        (r.body["ok"].as_bool(), r.body["broken_at"].as_i64()),
        (Some(false), Some(ids[1]))
    );
    // 删掉一条：下一条的 prev_hash 对不上
    tamper(
        &db,
        "UPDATE order_messages SET body = '明天发货' WHERE id = $1",
        ids[1],
    )
    .await;
    tamper(&db, "DELETE FROM order_messages WHERE id = $1", ids[2]).await;
    let r = call(&app, "GET", &verify, None, &ca).await;
    assert_eq!(r.body["broken_at"], ids[3]);

    // 没有私信谈判记录的订单
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/orders/{order}/negotiation"),
        None,
        &ca,
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn negotiation_evidence_stops_at_payment(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let (a, ca) = user(&app, &db).await;
    let (b, cb) = user(&app, &db).await;
    let (_, cs) = user(&app, &db).await;
    let conv = open_conversation(&app, &ca, b).await;
    say(&app, &ca, conv, "便宜点").await;
    let before_pay = say(&app, &cb, conv, "45 吧").await;
    let order = seed_order(&db, a, b).await;
    // indexer 付款上链时写入（这里直接写同样的行）
    let offer: i64 = sqlx::query_scalar(
        "INSERT INTO offers (conversation_id, seller_id, buyer_id, chain_id, verifying_contract, seller, buyer, token,
           amount, fee_payer, max_fee_bps, max_base_gas, max_min_fee, bond, late_slash_bps, instant, ship_window,
           confirm_window, listing_hash, expiry, max_fills, nonce, signature, offer_hash, status, expires_at)
         SELECT $1, $2, $3, chain_id, contract, seller, buyer, token, 5000000, 0, 100, 100000, 1000000, 0, 0, false,
           86400, 86400, listing_hash, 1800000000, 1, 1, '\\x01', offer_hash, 'paid', now() + interval '1 day'
         FROM escrow_orders WHERE id = $4
         RETURNING id",
    )
    .bind(conv)
    .bind(b)
    .bind(a)
    .bind(order)
    .fetch_one(&db)
    .await
    .unwrap();
    assert!(
        forum_db::order_chat::link_negotiation(&db, order)
            .await
            .unwrap()
    );
    assert!(
        !forum_db::order_chat::link_negotiation(&db, order)
            .await
            .unwrap(),
        "first link wins"
    );
    say(&app, &ca, conv, "付款后的消息").await;

    let r = call(&app, "GET", &format!("/api/v1/orders/{order}"), None, &ca).await;
    assert_eq!(
        r.body["negotiation"],
        json!({"conversation_id": conv, "last_message_id": before_pay["id"]})
    );
    assert_eq!(r.body["source"]["offer_id"], offer);
    let path = format!("/api/v1/orders/{order}/negotiation");
    let r = call(&app, "GET", &path, None, &cb).await;
    assert_eq!(bodies(&r.body), ["便宜点", "45 吧"]);
    let r = call(
        &app,
        "GET",
        &format!("{path}?after={}", before_pay["id"]),
        None,
        &cb,
    )
    .await;
    assert!(bodies(&r.body).is_empty());
    assert_eq!(
        call(&app, "GET", &path, None, &cs).await.status,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn notifications_page_read_and_unread(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let (a, ca) = user(&app, &db).await;
    let (b, cb) = user(&app, &db).await;
    let items: Vec<_> = (0..25)
        .map(|i| forum_db::notifications::NewNotification {
            user_id: a,
            kind: forum_db::notifications::KIND_REPLY,
            actor_id: Some(b),
            payload: json!({"topic_id": 1, "post_id": i, "floor": i + 2}),
        })
        .collect();
    let rows = forum_db::notifications::insert(&db, &items).await.unwrap();
    let ids: Vec<i64> = rows.iter().map(|(_, n)| n.id).collect();

    assert_eq!(
        call(&app, "GET", "/api/v1/notifications", None, &[])
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    let r = call(&app, "GET", "/api/v1/notifications/unread", None, &ca).await;
    assert_eq!(r.body["count"], 25);
    let r = call(&app, "GET", "/api/v1/notifications", None, &ca).await;
    let page = r.body["notifications"].as_array().unwrap();
    assert_eq!(page.len(), 20);
    assert_eq!(page[0]["id"], ids[24], "newest first");
    assert_eq!(page[0]["kind"], "reply");
    assert_eq!(page[0]["actor_id"], b);
    assert_eq!(page[0]["read"], false);
    let cursor = r.body["next_cursor"].as_i64().unwrap();
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/notifications?before={cursor}"),
        None,
        &ca,
    )
    .await;
    assert_eq!(r.body["notifications"].as_array().unwrap().len(), 5);
    assert_eq!(r.body["next_cursor"], Value::Null);
    // 重连补拉：之后的按时间正序
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/notifications?after={}", ids[21]),
        None,
        &ca,
    )
    .await;
    let after: Vec<i64> = r.body["notifications"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["id"].as_i64().unwrap())
        .collect();
    assert_eq!(after, ids[22..]);

    // 标为已读：水位只前进、不超过自己最新一条
    let read = |up_to: i64| {
        let app = app.clone();
        let ca = ca.clone();
        async move {
            call(
                &app,
                "POST",
                "/api/v1/notifications/read",
                Some(json!({"up_to_id": up_to})),
                &ca,
            )
            .await
            .body
        }
    };
    assert_eq!(
        read(ids[9]).await,
        json!({"count": 15, "last_read_id": ids[9]})
    );
    assert_eq!(read(ids[3]).await["last_read_id"], ids[9]);
    let r = call(&app, "GET", "/api/v1/notifications", None, &ca).await;
    assert_eq!(r.body["notifications"][14]["read"], false);
    assert_eq!(r.body["notifications"][15]["read"], true);
    assert_eq!(
        read(i64::MAX).await,
        json!({"count": 0, "last_read_id": ids[24]})
    );
    // 别人的通知不受影响
    let r = call(&app, "GET", "/api/v1/notifications", None, &cb).await;
    assert!(r.body["notifications"].as_array().unwrap().is_empty());
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn presence_respects_hidden_users_except_counterparties(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let (a, ca) = user(&app, &db).await;
    let (b, _) = user(&app, &db).await;
    let (_, cs) = user(&app, &db).await;
    sqlx::query(
        "UPDATE users SET show_online = false, last_seen_at = '2026-10-19T08:30:00Z' WHERE id = $1",
    )
    .bind(b)
    .execute(&db)
    .await
    .unwrap();
    let path = format!("/api/v1/users/presence?ids={b},{a},{b},999999");
    let r = call(&app, "GET", &path, None, &cs).await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert_eq!(
        r.body,
        json!([
            {"user_id": b, "online": false, "last_seen_at": null, "last_seen_date": "2026-10-19"},
            {"user_id": a, "online": false, "last_seen_at": null, "last_seen_date": null},
        ])
    );
    // 进行中订单的交易对手看到真实时间
    seed_order(&db, a, b).await;
    let r = call(&app, "GET", &path, None, &ca).await;
    assert_eq!(r.body[0]["last_seen_at"], "2026-10-19T08:30:00Z");
    for bad in [
        "",
        "1,x",
        &(0..101)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(","),
    ] {
        let r = call(
            &app,
            "GET",
            &format!("/api/v1/users/presence?ids={bad}"),
            None,
            &cs,
        )
        .await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{bad}");
    }
}

// ───────────────────────────── WebSocket ─────────────────────────────

async fn ws(addr: SocketAddr, cookies: &[String], origin: &str) -> Result<Ws, tungstenite::Error> {
    let mut req = format!("ws://{addr}/api/v1/ws")
        .into_client_request()
        .unwrap();
    req.headers_mut().insert("origin", origin.parse().unwrap());
    if !cookies.is_empty() {
        req.headers_mut()
            .insert("cookie", cookies.join("; ").parse().unwrap());
    }
    tokio_tungstenite::connect_async(req).await.map(|(s, _)| s)
}

fn rejected(r: Result<Ws, tungstenite::Error>) -> u16 {
    match r {
        Err(tungstenite::Error::Http(res)) => res.status().as_u16(),
        Err(e) => panic!("unexpected error {e}"),
        Ok(_) => panic!("connection unexpectedly accepted"),
    }
}

/// 下一条 `type` 为 `kind` 的推送（跳过其他类型），5 秒内必须到。
async fn expect(ws: &mut Ws, kind: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let msg = tokio::time::timeout(left, ws.next())
            .await
            .unwrap_or_else(|_| panic!("no {kind} event in time"))
            .unwrap()
            .unwrap();
        if let Message::Text(t) = msg {
            let v: Value = serde_json::from_str(t.as_str()).unwrap();
            if v["type"] == kind {
                return v;
            }
        }
    }
}

/// 一段时间内没有 `kind` 推送。
async fn silent(ws: &mut Ws, kind: &str) {
    let until = Instant::now() + Duration::from_millis(300);
    while let Ok(Some(Ok(msg))) =
        tokio::time::timeout(until.saturating_duration_since(Instant::now()), ws.next()).await
    {
        if let Message::Text(t) = msg {
            let v: Value = serde_json::from_str(t.as_str()).unwrap();
            assert_ne!(v["type"], kind, "unexpected {v}");
        }
    }
}

async fn send(ws: &mut Ws, v: Value) {
    ws.send(Message::Text(v.to_string().into())).await.unwrap();
}

/// 两个 forum-api 实例共用 Valkey：连在实例 1 上的用户收到在实例 2 上发出的私信、正在输入、已读与上下线。
#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn websocket_push_across_instances(db: PgPool) {
    let ns = namespace();
    let Some(s1) = state_with(db.clone(), wallet_state(), &ns) else {
        return;
    };
    let s2 = state_with(db.clone(), wallet_state(), &ns).unwrap();
    let (r1, r2) = (router_of(s1.clone()), router_of(s2.clone()));
    let (addr1, addr2) = (serve(s1).await, serve(s2).await);
    let (a, ca) = user(&r1, &db).await;
    let (b, cb) = user(&r2, &db).await;
    let conv = open_conversation(&r1, &ca, b).await;

    // 未登录、来源不对都不建连接
    assert_eq!(rejected(ws(addr1, &[], ORIGIN).await), 401);
    assert_eq!(rejected(ws(addr1, &ca, "https://evil.test").await), 403);

    let mut wa = ws(addr1, &ca, ORIGIN).await.unwrap();
    assert_eq!(expect(&mut wa, "ready").await["user_id"], a);
    send(
        &mut wa,
        json!({"type": "subscribe", "conversation_id": conv}),
    )
    .await;
    let sub = expect(&mut wa, "subscribed").await;
    assert_eq!(sub["presence"][0]["user_id"], b);
    assert_eq!(sub["presence"][0]["online"], false);
    // 别人的会话订阅不了
    send(
        &mut wa,
        json!({"type": "subscribe", "conversation_id": conv + 1000}),
    )
    .await;
    assert_eq!(expect(&mut wa, "error").await["code"], "NOT_FOUND");
    send(&mut wa, json!({"type": "nonsense"})).await;
    assert_eq!(expect(&mut wa, "error").await["code"], "INVALID_PARAMETER");

    // b 在实例 2 上线：a 订阅了与 b 的会话，收到上线
    let mut wb = ws(addr2, &cb, common::WALLET_ORIGIN).await.unwrap();
    expect(&mut wb, "ready").await;
    let p = expect(&mut wa, "presence").await;
    assert_eq!(
        (p["user_id"].as_i64(), p["online"].as_bool()),
        (Some(b), Some(true))
    );
    let r = call(
        &r1,
        "GET",
        &format!("/api/v1/users/presence?ids={b}"),
        None,
        &ca,
    )
    .await;
    assert_eq!(r.body[0]["online"], true);

    // 正在输入：只发给订阅了会话的对方连接，必须先订阅
    send(&mut wb, json!({"type": "typing", "conversation_id": conv})).await;
    assert_eq!(expect(&mut wb, "error").await["code"], "FORBIDDEN");
    send(
        &mut wb,
        json!({"type": "subscribe", "conversation_id": conv}),
    )
    .await;
    let sub = expect(&mut wb, "subscribed").await;
    assert_eq!(sub["presence"][0]["online"], true);
    send(&mut wb, json!({"type": "typing", "conversation_id": conv})).await;
    let t = expect(&mut wa, "typing").await;
    assert_eq!(
        (t["conversation_id"].as_i64(), t["user_id"].as_i64()),
        (Some(conv), Some(b))
    );
    silent(&mut wb, "typing").await;

    // b 经实例 2 发私信：实例 1 上的 a 收到，b 自己的连接也收到（多设备同步）
    let sent_at = Instant::now();
    let msg = say(&r2, &cb, conv, "在吗").await;
    let got = expect(&mut wa, "message").await;
    let latency = sent_at.elapsed();
    assert_eq!(got["message"], msg);
    assert!(latency < Duration::from_secs(1), "push took {latency:?}");
    assert_eq!(expect(&mut wb, "message").await["message"]["id"], msg["id"]);
    // 已读回执
    let r = call(
        &r1,
        "POST",
        &format!("/api/v1/conversations/{conv}/read"),
        Some(json!({"last_id": msg["id"]})),
        &ca,
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    let read = expect(&mut wb, "read").await;
    assert_eq!(
        (read["user_id"].as_i64(), read["last_id"].clone()),
        (Some(a), msg["id"].clone())
    );

    send(&mut wa, json!({"type": "ping"})).await;
    expect(&mut wa, "pong").await;

    // b 断开：最后一条连接没了，a 收到下线
    wb.close(None).await.unwrap();
    let p = expect(&mut wa, "presence").await;
    assert_eq!(
        (p["user_id"].as_i64(), p["online"].as_bool()),
        (Some(b), Some(false))
    );

    // 每个用户最多 5 条连接（两个实例合计）
    let mut extra = Vec::new();
    for i in 0..4 {
        let addr = if i % 2 == 0 { addr1 } else { addr2 };
        extra.push(ws(addr, &ca, ORIGIN).await.unwrap());
    }
    assert_eq!(rejected(ws(addr2, &ca, ORIGIN).await), 429);
    drop(extra);
}

/// 订单聊天推送：双方与订阅了订单的仲裁员收到；旁人订阅不了。
#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn order_chat_push_reaches_parties_and_subscribed_arbiters(db: PgPool) {
    let ns = namespace();
    let Some(s1) = state_with(db.clone(), wallet_state(), &ns) else {
        return;
    };
    let s2 = state_with(db.clone(), wallet_state(), &ns).unwrap();
    let (r1, r2) = (router_of(s1.clone()), router_of(s2.clone()));
    let (addr1, addr2) = (serve(s1).await, serve(s2).await);
    let (a, ca) = user(&r1, &db).await;
    let (b, cb) = user(&r1, &db).await;
    let (c, cc) = user(&r1, &db).await;
    let (_, cs) = user(&r1, &db).await;
    let order = seed_order(&db, a, b).await;
    forum_db::staff::grant(&db, c, forum_db::staff::ROLE_ADMIN, None, None)
        .await
        .unwrap();

    let mut wb = ws(addr1, &cb, ORIGIN).await.unwrap();
    expect(&mut wb, "ready").await;
    let mut wc = ws(addr2, &cc, ORIGIN).await.unwrap();
    expect(&mut wc, "ready").await;
    let mut ws_stranger = ws(addr2, &cs, ORIGIN).await.unwrap();
    expect(&mut ws_stranger, "ready").await;
    send(
        &mut ws_stranger,
        json!({"type": "subscribe", "order_id": order}),
    )
    .await;
    assert_eq!(expect(&mut ws_stranger, "error").await["code"], "NOT_FOUND");
    send(&mut wc, json!({"type": "subscribe", "order_id": order})).await;
    let sub = expect(&mut wc, "subscribed").await;
    let watched: Vec<i64> = sub["presence"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["user_id"].as_i64().unwrap())
        .collect();
    assert_eq!(watched.len(), 2);

    let r = call(
        &r2,
        "POST",
        &format!("/api/v1/orders/{order}/messages"),
        Some(json!({"body": "已发货"})),
        &ca,
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED);
    // 买家没订阅也收到（双方的所有连接）；订阅了的仲裁员收到
    assert_eq!(expect(&mut wb, "order_message").await["message"], r.body);
    assert_eq!(
        expect(&mut wc, "order_message").await["message"]["id"],
        r.body["id"]
    );
    silent(&mut ws_stranger, "order_message").await;
}
