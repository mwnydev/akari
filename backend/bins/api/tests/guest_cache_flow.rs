//! 游客响应缓存（plan 第 8 节「游客页面」）：命中、写操作精确失效、隐藏后不再返回旧内容、登录用户的响应绝不写入、
//! 冷启动时只有一个请求回源。需要 PostgreSQL（`DATABASE_URL`）与 Valkey（`VALKEY_URL`，未设置则跳过）。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

mod common;

use axum::Router;
use axum::http::StatusCode;
use common::{age, call, namespace, router_of, state_with, user, wallet_state};
use forum_api::guest::CACHE_HEADER;
use forum_db::PgPool;
use forum_db::forums::{self, NewForum};
use forum_db::staff;
use serde_json::{Value, json};

async fn forum(db: &PgPool, slug: &str) {
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
            allow_trade: false,
            color: None,
        },
    )
    .await
    .unwrap();
}

fn doc() -> Value {
    json!({"type": "doc", "content": []})
}

async fn member(app: &Router, db: &PgPool) -> (i64, Vec<String>) {
    let (id, cookies) = user(app, db).await;
    age(db, id).await;
    (id, cookies)
}

async fn new_topic(app: &Router, cookies: &[String], title: &str) -> i64 {
    let r = call(
        app,
        "POST",
        "/api/v1/forums/g/topics",
        Some(json!({"title": title, "html": "<p>正文</p>", "source": doc()})),
        cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    r.body["topic_id"].as_i64().unwrap()
}

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

/// 游客请求：返回 `(状态, x-guest-cache, 响应体)`。
async fn guest(app: &Router, uri: &str) -> (StatusCode, String, Value) {
    as_user(app, uri, &[]).await
}

async fn as_user(app: &Router, uri: &str, cookies: &[String]) -> (StatusCode, String, Value) {
    let r = call(app, "GET", uri, None, cookies).await;
    let cache = r
        .headers
        .get(CACHE_HEADER)
        .map(|v| v.to_str().unwrap().to_owned())
        .unwrap_or_default();
    (r.status, cache, r.body)
}

/// 本用例命名空间下的游客缓存条目（不含版本号与锁）。
async fn guest_entries(state: &forum_api::AppState) -> Vec<String> {
    let mut conn = state.kv.get().await.unwrap();
    let keys: Vec<String> = redis::cmd("KEYS")
        .arg(format!("{}g:*", state.cache.ns()))
        .query_async(&mut conn)
        .await
        .unwrap();
    keys.into_iter().filter(|k| !k.ends_with(":lock")).collect()
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn guest_pages_hit_and_writes_invalidate_precisely(db: PgPool) {
    let Some(state) = state_with(db.clone(), wallet_state(), &namespace()) else {
        return;
    };
    let app = router_of(state);
    forum(&db, "g").await;
    let (_, a) = member(&app, &db).await;
    let (_, b) = member(&app, &db).await;
    let t1 = new_topic(&app, &a, "第一帖").await;
    let t2 = new_topic(&app, &a, "第二帖").await;
    let page = |t: i64| format!("/api/v1/topics/{t}");

    for uri in [
        page(t1),
        "/api/v1/forums/g/topics".to_owned(),
        "/api/v1/topics/latest".to_owned(),
        "/api/v1/forums".to_owned(),
        "/api/v1/forums/g/topics?sort=hot".to_owned(),
        "/api/v1/topics/latest?sort=hot".to_owned(),
    ] {
        let (s, c, first) = guest(&app, &uri).await;
        assert_eq!((s, c.as_str()), (StatusCode::OK, "miss"), "{uri}");
        let (_, c, again) = guest(&app, &uri).await;
        assert_eq!(c, "hit", "{uri}");
        assert_eq!(first, again, "{uri}");
    }
    guest(&app, &page(t2)).await;

    // 回帖：该主题、板块与首页失效；别的主题不受影响
    let post = reply(&app, &b, t1).await;
    let (_, c, body) = guest(&app, &page(t1)).await;
    assert_eq!(c, "miss");
    assert_eq!(body["posts"].as_array().unwrap().len(), 2);
    assert_eq!(guest(&app, &page(t2)).await.1, "hit");
    let (_, c, body) = guest(&app, "/api/v1/forums/g/topics").await;
    assert_eq!(c, "miss");
    assert_eq!(
        body["topics"][0]["id"], t1,
        "reply bumps the topic to the top"
    );
    assert_eq!(guest(&app, "/api/v1/topics/latest").await.1, "miss");
    assert_eq!(guest(&app, "/api/v1/forums").await.1, "hit");
    // 热门第一页与最新列表依赖同样的版本号；热门的游标页不缓存
    assert_eq!(
        guest(&app, "/api/v1/forums/g/topics?sort=hot").await.1,
        "miss"
    );
    assert_eq!(
        guest(&app, "/api/v1/topics/latest?sort=hot").await.1,
        "miss"
    );
    assert_eq!(
        guest(&app, "/api/v1/topics/latest?sort=hot&cursor=1.5_1")
            .await
            .1,
        "bypass"
    );

    // 点赞（不在第 1 页缓存里，但在游客页里）
    let r = call(&app, "PUT", &format!("/api/v1/posts/{post}/like"), None, &a).await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    let (_, c, body) = guest(&app, &page(t1)).await;
    assert_eq!(c, "miss");
    assert_eq!(body["social"][1]["likes"], 1);

    // 编辑 1 楼标题：主题页与板块列表都失效
    let first_post = body["posts"][0]["id"].as_i64().unwrap();
    let r = call(
        &app,
        "PATCH",
        &format!("/api/v1/posts/{first_post}"),
        Some(json!({"html": "<p>改过</p>", "source": doc(), "title": "改过的标题"})),
        &a,
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{:?}", r.body);
    let (_, c, body) = guest(&app, "/api/v1/forums/g/topics").await;
    assert_eq!(c, "miss");
    assert_eq!(body["topics"][0]["title"], "改过的标题");
    assert_eq!(guest(&app, &page(t1)).await.1, "miss");
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn hidden_topic_is_never_served_from_cache(db: PgPool) {
    let Some(state) = state_with(db.clone(), wallet_state(), &namespace()) else {
        return;
    };
    let app = router_of(state.clone());
    forum(&db, "g").await;
    let (_, a) = member(&app, &db).await;
    let (mod_id, m) = member(&app, &db).await;
    staff::grant(&db, mod_id, staff::ROLE_MODERATOR, Some(1), None)
        .await
        .unwrap();
    let t = new_topic(&app, &a, "要隐藏的").await;
    let uri = format!("/api/v1/topics/{t}");
    guest(&app, &uri).await;
    assert_eq!(guest(&app, &uri).await.1, "hit");
    let (_, _, board) = guest(&app, "/api/v1/forums/g/topics").await;
    assert_eq!(board["topics"][0]["id"], t);

    let r = call(
        &app,
        "POST",
        &format!("/api/v1/topics/{t}/moderation"),
        Some(json!({"action": "hide", "reason": "测试"})),
        &m,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert_eq!(guest(&app, &uri).await.0, StatusCode::NOT_FOUND);
    // 回源发现已不可见时连同旧条目一起删除：之后的请求也不会拿到过期的旧页面
    assert!(
        !guest_entries(&state)
            .await
            .iter()
            .any(|k| k.ends_with(&format!("g:topic:{t}:1"))),
        "stale entry kept"
    );
    assert_eq!(guest(&app, &uri).await.0, StatusCode::NOT_FOUND);
    let (_, c, board) = guest(&app, "/api/v1/forums/g/topics").await;
    assert_eq!(c, "miss");
    assert!(board["topics"].as_array().unwrap().is_empty());
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn logged_in_responses_are_never_cached(db: PgPool) {
    let Some(state) = state_with(db.clone(), wallet_state(), &namespace()) else {
        return;
    };
    let app = router_of(state.clone());
    forum(&db, "g").await;
    let (_, a) = member(&app, &db).await;
    let t = new_topic(&app, &a, "登录用户").await;
    let uri = format!("/api/v1/topics/{t}");
    for u in [
        uri.as_str(),
        "/api/v1/forums/g/topics",
        "/api/v1/topics/latest",
        "/api/v1/forums",
    ] {
        let (s, c, _) = as_user(&app, u, &a).await;
        assert_eq!((s, c.as_str()), (StatusCode::OK, "bypass"), "{u}");
    }
    assert!(
        guest_entries(&state).await.is_empty(),
        "logged-in response stored"
    );

    // 游客写入的条目也不会返回给登录用户：登录用户的页面带着按用户计算的字段
    assert_eq!(guest(&app, &uri).await.1, "miss");
    let (_, c, body) = as_user(&app, &uri, &a).await;
    assert_eq!(c, "bypass");
    assert_eq!(body["can_reply"], true);
    assert_eq!(guest(&app, &uri).await.2["can_reply"], false);
    // 伪造的会话 Cookie 当游客处理（拿到的是游客内容）
    let (_, c, body) = as_user(&app, &uri, &["sid=forged".to_owned()]).await;
    assert_eq!(c, "hit");
    assert_eq!(body["can_reply"], false);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn cold_entry_is_built_once(db: PgPool) {
    let Some(state) = state_with(db.clone(), wallet_state(), &namespace()) else {
        return;
    };
    let app = router_of(state);
    forum(&db, "g").await;
    let (_, a) = member(&app, &db).await;
    let t = new_topic(&app, &a, "热门").await;
    let uri = format!("/api/v1/topics/{t}");
    let tasks: Vec<_> = (0..24)
        .map(|_| {
            let app = app.clone();
            let uri = uri.clone();
            tokio::spawn(async move { guest(&app, &uri).await })
        })
        .collect();
    let mut kinds = Vec::new();
    for t in tasks {
        let (s, c, _) = t.await.unwrap();
        assert_eq!(s, StatusCode::OK);
        kinds.push(c);
    }
    assert_eq!(
        kinds.iter().filter(|c| *c == "miss").count(),
        1,
        "{kinds:?}"
    );
    assert!(kinds.iter().all(|c| c == "miss" || c == "hit"), "{kinds:?}");
}
