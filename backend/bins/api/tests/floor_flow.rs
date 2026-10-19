//! 楼层点赞与朋友圈式评论、举报、主题列表的信息行与楼层时间轴。
//! 需要 PostgreSQL（`DATABASE_URL`）与 Valkey（`VALKEY_URL`，未设置则跳过）。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

mod common;

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use axum::Router;
use axum::http::StatusCode;
use common::{
    age, build, call, namespace, router_of, serve, set_username, state_with, user, wallet_state,
};
use forum_api::moderation::{self, RevertEffects, RevertStatus};
use forum_db::PgPool;
use forum_db::forums::{self, NewForum};
use forum_db::staff;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn forum(db: &PgPool, slug: &str) -> i32 {
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

fn doc() -> Value {
    json!({"type": "doc"})
}

/// 发主题，返回 `(主题 ID, 1 楼 ID)`。
async fn topic(app: &Router, cookies: &[String], slug: &str) -> (i64, i64) {
    let r = call(
        app,
        "POST",
        &format!("/api/v1/forums/{slug}/topics"),
        Some(json!({"title": "楼层社交", "html": "<p>1 楼</p>", "source": doc()})),
        cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    (
        r.body["topic_id"].as_i64().unwrap(),
        r.body["post_id"].as_i64().unwrap(),
    )
}

/// 回帖，返回楼层 ID。
async fn reply(app: &Router, cookies: &[String], topic: i64) -> i64 {
    let r = call(
        app,
        "POST",
        &format!("/api/v1/topics/{topic}/posts"),
        Some(json!({"html": "<p>回复</p>", "source": doc()})),
        cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    r.body["post_id"].as_i64().unwrap()
}

async fn member(app: &Router, db: &PgPool) -> (i64, Vec<String>) {
    let (id, cookies) = user(app, db).await;
    age(db, id).await;
    (id, cookies)
}

async fn comment(app: &Router, cookies: &[String], post: i64, body: Value) -> common::Res {
    call(
        app,
        "POST",
        &format!("/api/v1/posts/{post}/comments"),
        Some(body),
        cookies,
    )
    .await
}

async fn like(app: &Router, cookies: &[String], post: i64, method: &str) -> common::Res {
    call(
        app,
        method,
        &format!("/api/v1/posts/{post}/like"),
        None,
        cookies,
    )
    .await
}

async fn page(app: &Router, cookies: &[String], topic: i64) -> Value {
    let r = call(
        app,
        "GET",
        &format!("/api/v1/topics/{topic}"),
        None,
        cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    r.body
}

async fn outbox(db: &PgPool, kind: &str) -> Vec<Value> {
    sqlx::query_scalar("SELECT payload FROM outbox WHERE kind = $1 ORDER BY id")
        .bind(kind)
        .fetch_all(db)
        .await
        .unwrap()
}

async fn sanction(db: &PgPool, kind: i16, user_id: i64, by: i64) {
    sqlx::query(
        "INSERT INTO sanctions (kind, user_id, reason, created_by) VALUES ($1, $2, 'test', $3)",
    )
    .bind(kind)
    .bind(user_id)
    .bind(by)
    .execute(db)
    .await
    .unwrap();
}

fn has_user(body: &Value, id: i64) -> bool {
    body["users"]
        .as_array()
        .unwrap()
        .iter()
        .any(|u| u["id"] == id)
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn likes_are_idempotent_counted_and_digested(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    forum(&db, "general").await;
    let (alice, a) = member(&app, &db).await;
    let (bob, b) = member(&app, &db).await;
    let (carol, c) = member(&app, &db).await;
    let (t, p1) = topic(&app, &a, "general").await;

    assert_eq!(
        like(&app, &[], p1, "PUT").await.status,
        StatusCode::UNAUTHORIZED
    );
    for _ in 0..2 {
        let r = like(&app, &b, p1, "PUT").await;
        assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
        assert_eq!(r.body, json!({"liked": true, "likes": 1}));
    }
    let r = like(&app, &c, p1, "PUT").await;
    assert_eq!(r.body["likes"], 2);
    // 楼主给自己点赞照常计数，但不通知自己
    assert_eq!(like(&app, &a, p1, "PUT").await.body["likes"], 3);

    // 窗口内的赞只排一条延迟投递的合并通知
    let digests: Vec<(Value, bool)> = sqlx::query_as(
        "SELECT payload, next_attempt_at > now() + interval '4 minutes' FROM outbox WHERE kind = 'notify.likes'",
    )
    .fetch_all(&db)
    .await
    .unwrap();
    assert_eq!(digests, vec![(json!({"post_id": p1}), true)]);

    // 主题页：计数、最近点赞的人（新的在前）与访问者自己的状态；名片里有点赞的人
    let body = page(&app, &b, t).await;
    let s = &body["social"][0];
    assert_eq!(s["post_id"], p1);
    assert_eq!(s["likes"], 3);
    assert_eq!(s["liked"], true);
    assert_eq!(s["likers"], json!([alice, carol, bob]));
    assert!(has_user(&body, carol));
    assert_eq!(page(&app, &[], t).await["social"][0]["liked"], false);

    // 取消赞：幂等
    for _ in 0..2 {
        let r = like(&app, &b, p1, "DELETE").await;
        assert_eq!(r.status, StatusCode::OK);
        assert_eq!(r.body, json!({"liked": false, "likes": 2}));
    }
    assert_eq!(page(&app, &b, t).await["social"][0]["liked"], false);

    // 已删除的楼层不能点赞，主题页上也不再显示赞
    let p2 = reply(&app, &b, t).await;
    assert_eq!(like(&app, &c, p2, "PUT").await.status, StatusCode::OK);
    let r = call(&app, "DELETE", &format!("/api/v1/posts/{p2}"), None, &b).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert_eq!(
        like(&app, &a, p2, "PUT").await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(page(&app, &[], t).await["social"][1]["likes"], 0);

    // 被封号的不能点赞
    sanction(&db, forum_db::sanctions::KIND_BAN, carol, alice).await;
    let r = like(&app, &c, p1, "PUT").await;
    assert_eq!(r.body["code"], "ACCOUNT_BANNED");
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn comments_notify_page_and_sanitise(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    forum(&db, "general").await;
    let (alice, a) = member(&app, &db).await;
    let (bob, b) = member(&app, &db).await;
    let (carol, c) = member(&app, &db).await;
    set_username(&db, carol, "carol_c").await;
    let (t, p1) = topic(&app, &a, "general").await;

    // 纯文本：控制字符换成空格，HTML 原样当文字保存；@ 已存在的用户名会通知
    let r = comment(
        &app,
        &b,
        p1,
        json!({"body": " <b>沙发</b>\n@Carol_C @nobody "}),
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    let c1 = r.body["comment"]["id"].as_i64().unwrap();
    assert_eq!(r.body["comment"]["body"], "<b>沙发</b> @Carol_C @nobody");
    assert_eq!(r.body["comment"]["reply_to_id"], Value::Null);
    assert_eq!(r.body["comment_count"], 1);
    for bad in [json!({"body": " \n "}), json!({"body": "字".repeat(501)})] {
        assert_eq!(
            comment(&app, &b, p1, bad).await.status,
            StatusCode::BAD_REQUEST
        );
    }

    // 楼主回复 bob 的评论：bob 收到「回复了你的评论」，楼主自己不收；carol 回复楼主（楼主只收一条）
    let r = comment(&app, &a, p1, json!({"body": "谢谢", "reply_to": c1})).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    assert_eq!(r.body["comment"]["reply_to_id"], bob);
    let c2 = r.body["comment"]["id"].as_i64().unwrap();
    let r = comment(&app, &c, p1, json!({"body": "+1", "reply_to": c2})).await;
    assert_eq!(r.body["comment"]["reply_to_id"], alice);
    let notes: Vec<(i64, i64, bool)> = outbox(&db, "notify.comment")
        .await
        .iter()
        .map(|p| {
            (
                p["to"].as_i64().unwrap(),
                p["from"].as_i64().unwrap(),
                p["reply"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        notes,
        vec![
            (alice, bob, false),
            (bob, alice, true),
            (alice, carol, true)
        ]
    );
    let mentions = outbox(&db, "notify.mention").await;
    assert_eq!(mentions.len(), 1);
    assert_eq!(
        (
            mentions[0]["to"].as_i64(),
            mentions[0]["comment_id"].as_i64()
        ),
        (Some(carol), Some(c1))
    );

    // 回复的评论必须在同一楼
    let p2 = reply(&app, &b, t).await;
    let r = comment(&app, &c, p2, json!({"body": "x", "reply_to": c1})).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);

    // 主题页每楼只带最早 5 条，其余分页取
    for i in 0..5 {
        let r = comment(&app, &c, p1, json!({"body": format!("第 {i} 条")})).await;
        assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    }
    let body = page(&app, &[], t).await;
    let s = &body["social"][0];
    assert_eq!(s["comment_count"], 8);
    let shown = s["comments"].as_array().unwrap();
    assert_eq!(shown.len(), 5);
    assert_eq!(shown[0]["id"], c1);
    assert!(has_user(&body, bob) && has_user(&body, carol));
    let after = shown[4]["id"].as_i64().unwrap();
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/posts/{p1}/comments?after={after}"),
        None,
        &[],
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    let rest = r.body["comments"].as_array().unwrap();
    assert_eq!(rest.len(), 3);
    assert_eq!(rest[2]["body"], "第 4 条");
    assert_eq!(r.body["next_cursor"], Value::Null);

    // 作者删除自己的评论：计数减一，不写审计日志；别人删不了（没有管理权限）
    let r = call(&app, "DELETE", &format!("/api/v1/comments/{c1}"), None, &c).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let r = call(&app, "DELETE", &format!("/api/v1/comments/{c1}"), None, &b).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    let r = call(&app, "DELETE", &format!("/api/v1/comments/{c1}"), None, &b).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    let body = page(&app, &[], t).await;
    assert_eq!(body["social"][0]["comment_count"], 7);
    assert_ne!(body["social"][0]["comments"][0]["id"], c1);
    let audits: i64 = sqlx::query_scalar("SELECT count(*) FROM admin_audit_logs")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(audits, 0);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn comment_permissions_sanctions_and_moderation(db: PgPool) {
    let ns = namespace();
    let Some(state) = state_with(db.clone(), wallet_state(), &ns) else {
        return;
    };
    let app = router_of(state.clone());
    let general = forum(&db, "general").await;
    let other = forum(&db, "other").await;
    let (alice, a) = member(&app, &db).await;
    let (bob, b) = member(&app, &db).await;
    let (mod_id, m) = member(&app, &db).await;
    let (other_mod, om) = member(&app, &db).await;
    staff::grant(&db, mod_id, staff::ROLE_MODERATOR, Some(general), None)
        .await
        .unwrap();
    staff::grant(&db, other_mod, staff::ROLE_MODERATOR, Some(other), None)
        .await
        .unwrap();
    let (t, p1) = topic(&app, &a, "general").await;
    let c1 = comment(&app, &b, p1, json!({"body": "广告"})).await.body["comment"]["id"]
        .as_i64()
        .unwrap();

    // 游客不能评论；版主删除他人评论要填理由，别的板块的版主不行
    assert_eq!(
        comment(&app, &[], p1, json!({"body": "x"})).await.status,
        StatusCode::UNAUTHORIZED
    );
    let del = |cookies: Vec<String>, q: &'static str| {
        let app = app.clone();
        async move {
            call(
                &app,
                "DELETE",
                &format!("/api/v1/comments/{c1}{q}"),
                None,
                &cookies,
            )
            .await
        }
    };
    assert_eq!(
        del(om.clone(), "?reason=x").await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(del(m.clone(), "").await.status, StatusCode::BAD_REQUEST);
    let r = del(m.clone(), "?reason=%E5%B9%BF%E5%91%8A").await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{:?}", r.body);
    let (action, target, reason): (String, String, String) =
        sqlx::query_as("SELECT action, target, reason FROM admin_audit_logs WHERE actor_id = $1")
            .bind(mod_id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(
        (action.as_str(), target.as_str(), reason.as_str()),
        ("comment.delete", &*format!("comment:{c1}"), "广告")
    );
    assert_eq!(page(&app, &[], t).await["social"][0]["comment_count"], 0);

    // 恢复（管理操作）后又看得到；审计日志一键撤销把恢复再撤掉
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/comments/{c1}/restore"),
        Some(json!({"reason": "误删"})),
        &m,
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{:?}", r.body);
    assert_eq!(page(&app, &[], t).await["social"][0]["comment_count"], 1);
    let restore = forum_db::audit::list(&db, Some(mod_id), None, 1)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(restore.action, "comment.restore");
    let mut tx = db.begin().await.unwrap();
    let mut effects = RevertEffects::default();
    let status = moderation::revert(&state, &mut tx, alice, &restore, &mut effects)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    // 撤销接口提交后执行的缓存维护（游客缓存里的主题页带着评论）
    effects.apply(&state).await;
    assert_eq!(status, RevertStatus::Reverted);
    assert_eq!(page(&app, &[], t).await["social"][0]["comment_count"], 0);

    // 被禁言（本板块）或封号不能评论
    sanction(&db, forum_db::sanctions::KIND_MUTE, bob, mod_id).await;
    let r = comment(&app, &b, p1, json!({"body": "x"})).await;
    assert_eq!(
        (r.status, r.body["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("MUTED"))
    );
    let (carol, c) = member(&app, &db).await;
    sanction(&db, forum_db::sanctions::KIND_BAN, carol, alice).await;
    let r = comment(&app, &c, p1, json!({"body": "x"})).await;
    assert_eq!(r.body["code"], "ACCOUNT_BANNED");

    // 锁定的主题：普通用户不能评论，版主可以
    sqlx::query("UPDATE topics SET flags = 4 WHERE id = $1")
        .bind(t)
        .execute(&db)
        .await
        .unwrap();
    let (_, d) = member(&app, &db).await;
    assert_eq!(
        comment(&app, &d, p1, json!({"body": "x"})).await.body["code"],
        "TOPIC_LOCKED"
    );
    assert_eq!(
        comment(&app, &m, p1, json!({"body": "注意"})).await.status,
        StatusCode::CREATED
    );

    // 隐藏的主题：评论、翻页、举报对非管理人员都是不存在
    sqlx::query("UPDATE topics SET flags = 8 WHERE id = $1")
        .bind(t)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(
        comment(&app, &d, p1, json!({"body": "x"})).await.status,
        StatusCode::NOT_FOUND
    );
    let list = format!("/api/v1/posts/{p1}/comments");
    assert_eq!(
        call(&app, "GET", &list, None, &d).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&app, "GET", &list, None, &m).await.status,
        StatusCode::OK
    );
    assert_eq!(
        like(&app, &d, p1, "PUT").await.status,
        StatusCode::NOT_FOUND
    );

    // 已删除楼层的评论只有管理人员能看
    sqlx::query("UPDATE topics SET flags = 0 WHERE id = $1")
        .bind(t)
        .execute(&db)
        .await
        .unwrap();
    let r = call(
        &app,
        "DELETE",
        &format!("/api/v1/posts/{p1}?reason=x"),
        None,
        &m,
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert_eq!(
        call(&app, "GET", &list, None, &d).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&app, "GET", &list, None, &m).await.status,
        StatusCode::OK
    );
    let body = page(&app, &d, t).await;
    assert_eq!(body["social"][0]["comments"], json!([]));
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn new_users_share_the_posting_limit(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    forum(&db, "general").await;
    let (_, a) = member(&app, &db).await;
    let (_, p1) = topic(&app, &a, "general").await;
    let (_, fresh) = user(&app, &db).await;
    for i in 0..5 {
        let r = comment(&app, &fresh, p1, json!({"body": format!("{i}")})).await;
        assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    }
    let r = comment(&app, &fresh, p1, json!({"body": "6"})).await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn reports_are_recorded_once(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let general = forum(&db, "general").await;
    let (alice, a) = member(&app, &db).await;
    let (bob, b) = member(&app, &db).await;
    let (_, p1) = topic(&app, &a, "general").await;
    let c1 = comment(&app, &a, p1, json!({"body": "x"})).await.body["comment"]["id"]
        .as_i64()
        .unwrap();
    for _ in 0..2 {
        let r = call(
            &app,
            "POST",
            &format!("/api/v1/posts/{p1}/report"),
            Some(json!({})),
            &b,
        )
        .await;
        assert_eq!(r.status, StatusCode::NO_CONTENT, "{:?}", r.body);
        let r = call(
            &app,
            "POST",
            &format!("/api/v1/comments/{c1}/report"),
            Some(json!({"reason": " 引战 "})),
            &b,
        )
        .await;
        assert_eq!(r.status, StatusCode::NO_CONTENT, "{:?}", r.body);
    }
    let rows: Vec<(i16, i64, i32, String)> = sqlx::query_as(
        "SELECT target_kind, target_id, forum_id, reason FROM reports WHERE reporter_id = $1 ORDER BY id",
    )
    .bind(bob)
    .fetch_all(&db)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![
            (1, p1, general, String::new()),
            (2, c1, general, "引战".to_owned())
        ]
    );
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/posts/{p1}/report"),
        Some(json!({"reason": "长".repeat(501)})),
        &b,
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("/api/v1/posts/{p1}/report"),
            Some(json!({})),
            &[]
        )
        .await
        .status,
        StatusCode::UNAUTHORIZED
    );
    sanction(&db, forum_db::sanctions::KIND_BAN, bob, alice).await;
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/comments/{c1}/report"),
        Some(json!({})),
        &b,
    )
    .await;
    assert_eq!(r.body["code"], "ACCOUNT_BANNED");
}

/// 交易帖与它的一张公开报价（可成交 `fills` 次）和一张给 `buyer` 的指定买家报价；返回交易帖 ID。
async fn listing_with_offers(db: &PgPool, topic: i64, seller: i64, buyer: i64, fills: i64) -> i64 {
    sqlx::query("UPDATE topics SET kind = 1 WHERE id = $1")
        .bind(topic)
        .execute(db)
        .await
        .unwrap();
    let listing: i64 = sqlx::query_scalar(
        "INSERT INTO trade_listings (topic_id, seller_id, chain_id, price, terms, content_hash, cny_per_usdt)
         VALUES ($1, $2, 421614, 45000000, '{\"instant\": true}', decode(repeat('ab', 32), 'hex'), 72000)
         RETURNING id",
    )
    .bind(topic)
    .bind(seller)
    .fetch_one(db)
    .await
    .unwrap();
    for (n, max_fills, buyer) in [(1_i32, fills, None), (2, 5, Some(buyer))] {
        sqlx::query(
            "INSERT INTO offers (listing_id, seller_id, buyer_id, chain_id, verifying_contract, seller, buyer, token,
               amount, fee_payer, max_fee_bps, max_base_gas, max_min_fee, bond, late_slash_bps, instant, ship_window,
               confirm_window, listing_hash, expiry, max_fills, nonce, signature, offer_hash, expires_at)
             VALUES ($1, $2, $3, 421614, decode(repeat('44', 20), 'hex'), decode(repeat('11', 20), 'hex'),
               decode(repeat('00', 20), 'hex'), decode(repeat('33', 20), 'hex'), 45000000, 0, 100, 100000, 1000000,
               0, 0, true, 0, 86400, decode(repeat('ab', 32), 'hex'), 1800000000, $4, $5, '\\x01',
               sha256(int4send($5)), now() + interval '1 day')",
        )
        .bind(listing)
        .bind(seller)
        .bind(buyer)
        .bind(max_fills)
        .bind(n)
        .execute(db)
        .await
        .unwrap();
    }
    listing
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn topic_list_rows_and_timeline(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    forum(&db, "general").await;
    let (alice, a) = member(&app, &db).await;
    let mut others = Vec::new();
    for _ in 0..5 {
        others.push(member(&app, &db).await);
    }
    let (t, _) = topic(&app, &a, "general").await;
    // 发言顺序：u0 u1 u2 楼主 u3 u4 u0 → 参与者 = 楼主 + 最近发言的 4 个不同的人
    for i in [0, 1, 2, usize::MAX, 3, 4, 0] {
        let cookies = if i == usize::MAX { &a } else { &others[i].1 };
        reply(&app, cookies, t).await;
    }
    let u = |i: usize| others[i].0;
    let listing = listing_with_offers(&db, t, alice, u(1), 3).await;

    let r = call(&app, "GET", "/api/v1/forums/general/topics", None, &[]).await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    let row = &r.body["topics"][0];
    assert_eq!(row["participants"], json!([alice, u(0), u(4), u(3), u(2)]));
    assert!(has_user(&r.body, u(2)));
    assert_eq!(
        (row["replies"].as_i64(), row["views"].as_i64()),
        (Some(7), Some(0))
    );
    assert_eq!(
        row["trade"],
        json!({"listing_id": listing, "price": "45000000", "cny_per_usdt": 72000, "instant": true, "stock": 3})
    );
    assert_eq!(row["auction"], Value::Null);

    // 公开报价成交一单后库存减一（回滚的订单不算）
    sqlx::query(
        "INSERT INTO chains (chain_id, name, escrow_address, config_address, token_address, token_symbol,
           token_decimals, start_block, safe_depth, finalized_depth)
         VALUES (421614, 'test', decode(repeat('44', 20), 'hex'), decode(repeat('c0', 20), 'hex'),
           decode(repeat('33', 20), 'hex'), 'USDT', 6, 0, 240, 4800)",
    )
    .execute(&db)
    .await
    .unwrap();
    for (n, state) in [(1_i64, 1_i16), (2, 0)] {
        sqlx::query(
            "INSERT INTO escrow_orders (chain_id, contract, chain_order_id, buyer, seller, token, amount, instant,
               offer_hash, listing_hash, state, level, created_block, updated_block)
             VALUES (421614, decode(repeat('44', 20), 'hex'), $1, decode(repeat('b1', 20), 'hex'),
               decode(repeat('11', 20), 'hex'), decode(repeat('33', 20), 'hex'), 45000000, true,
               sha256(int4send(1)), decode(repeat('ab', 32), 'hex'), $2, 'latest', 1, 1)",
        )
        .bind(n)
        .bind(state)
        .execute(&db)
        .await
        .unwrap();
    }
    let r = call(&app, "GET", "/api/v1/topics/latest", None, &[]).await;
    assert_eq!(r.body["topics"][0]["trade"]["stock"], 2);

    // 时间轴：楼层数、首尾时间与每层的时间
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/topics/{t}/timeline"),
        None,
        &[],
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert_eq!(r.body["last_floor"], 8);
    let points = r.body["points"].as_array().unwrap();
    assert_eq!(points.len(), 8);
    assert_eq!(
        (points[0]["floor"].as_i64(), points[7]["floor"].as_i64()),
        (Some(1), Some(8))
    );
    assert_eq!(points[7]["at"], r.body["last_post_at"]);
    sqlx::query("UPDATE topics SET flags = 8 WHERE id = $1")
        .bind(t)
        .execute(&db)
        .await
        .unwrap();
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/topics/{t}/timeline"),
        None,
        &[],
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

async fn ws(addr: SocketAddr, cookies: &[String]) -> Ws {
    let mut req = format!("ws://{addr}/api/v1/ws")
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert("origin", common::ORIGIN.parse().unwrap());
    req.headers_mut()
        .insert("cookie", cookies.join("; ").parse().unwrap());
    tokio_tungstenite::connect_async(req).await.unwrap().0
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

async fn send(ws: &mut Ws, v: Value) {
    ws.send(Message::Text(v.to_string().into())).await.unwrap();
}

/// 打开主题页的连接实时收到评论、删除评论与点赞；隐藏的主题订阅不了、也不再推送。
#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn topic_page_updates_live(db: PgPool) {
    let ns = namespace();
    let Some(state) = state_with(db.clone(), wallet_state(), &ns) else {
        return;
    };
    let app = router_of(state.clone());
    let addr = serve(state).await;
    forum(&db, "general").await;
    let (_, a) = member(&app, &db).await;
    let (bob, b) = member(&app, &db).await;
    let (t, p1) = topic(&app, &a, "general").await;

    let mut wa = ws(addr, &a).await;
    expect(&mut wa, "ready").await;
    send(&mut wa, json!({"type": "subscribe", "topic_id": t})).await;
    let sub = expect(&mut wa, "subscribed").await;
    assert_eq!(
        (sub["topic_id"].as_i64(), sub["presence"].clone()),
        (Some(t), json!([]))
    );
    send(&mut wa, json!({"type": "subscribe", "topic_id": 999_999})).await;
    assert_eq!(expect(&mut wa, "error").await["code"], "NOT_FOUND");
    send(&mut wa, json!({"type": "typing", "topic_id": t})).await;
    assert_eq!(expect(&mut wa, "error").await["code"], "INVALID_PARAMETER");

    let c1 = comment(&app, &b, p1, json!({"body": "实时"})).await.body["comment"]["id"]
        .as_i64()
        .unwrap();
    let e = expect(&mut wa, "comment").await;
    assert_eq!(
        (
            e["topic_id"].as_i64(),
            e["comment"]["id"].as_i64(),
            e["comment"]["body"].as_str(),
            e["comments"].as_i64()
        ),
        (Some(t), Some(c1), Some("实时"), Some(1))
    );
    like(&app, &b, p1, "PUT").await;
    let e = expect(&mut wa, "likes").await;
    assert_eq!(
        e,
        json!({"type": "likes", "topic_id": t, "post_id": p1, "user_id": bob, "liked": true, "likes": 1})
    );
    call(&app, "DELETE", &format!("/api/v1/comments/{c1}"), None, &b).await;
    let e = expect(&mut wa, "comment_removed").await;
    assert_eq!(
        (e["comment_id"].as_i64(), e["comments"].as_i64()),
        (Some(c1), Some(0))
    );

    // 主题被隐藏后：管理人员也不能再评论；管理人员取消赞照常但不推送
    // （之后在另一个主题里的评论先到、中间没有点赞推送，说明那条没有推）
    let (t2, p2) = topic(&app, &a, "general").await;
    send(&mut wa, json!({"type": "subscribe", "topic_id": t2})).await;
    expect(&mut wa, "subscribed").await;
    let (admin_id, m) = member(&app, &db).await;
    staff::grant(&db, admin_id, staff::ROLE_ADMIN, None, None)
        .await
        .unwrap();
    like(&app, &m, p1, "PUT").await;
    assert_eq!(expect(&mut wa, "likes").await["likes"], 2);
    sqlx::query("UPDATE topics SET flags = 8 WHERE id = $1")
        .bind(t)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(
        comment(&app, &m, p1, json!({"body": "内部"})).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(like(&app, &m, p1, "DELETE").await.body["likes"], 1);
    comment(&app, &b, p2, json!({"body": "公开"})).await;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let msg = tokio::time::timeout(left, wa.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Message::Text(raw) = msg {
            let v: Value = serde_json::from_str(raw.as_str()).unwrap();
            assert_ne!(v["type"], "likes", "hidden topic pushed {v}");
            if v["type"] == "comment" {
                assert_eq!(v["topic_id"], t2);
                break;
            }
        }
    }
    send(&mut wa, json!({"type": "subscribe", "topic_id": t})).await;
    assert_eq!(expect(&mut wa, "error").await["code"], "NOT_FOUND");
}
