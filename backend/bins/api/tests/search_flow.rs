//! 搜索接口：可见性（删除、隐藏、等级不够的板块）、高亮片段、主题与用户搜索、板块过滤、游标翻页、游客限流。
//! 需要 PostgreSQL（`DATABASE_URL`）与 Valkey（`VALKEY_URL`，未设置则跳过）。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

mod common;

use axum::Router;
use axum::http::StatusCode;
use common::{age, build, call, set_username, user};
use forum_db::PgPool;
use forum_db::forums::{self, NewForum};
use serde_json::{Value, json};

async fn forum(db: &PgPool, slug: &str, view: i16) {
    forums::create(
        db,
        &NewForum {
            parent_id: None,
            slug,
            name: slug,
            description: "",
            position: 0,
            view_min_level: view,
            post_min_level: 1,
            reply_min_level: 1,
            allow_trade: false,
            color: None,
        },
    )
    .await
    .unwrap();
}

async fn member(app: &Router, db: &PgPool, level: i16) -> (i64, Vec<String>) {
    let (id, cookies) = user(app, db).await;
    age(db, id).await;
    sqlx::query("INSERT INTO user_levels (user_id, level) VALUES ($1, $2)")
        .bind(id)
        .bind(level)
        .execute(db)
        .await
        .unwrap();
    (id, cookies)
}

async fn topic(app: &Router, cookies: &[String], board: &str, title: &str, text: &str) -> i64 {
    let r = call(
        app,
        "POST",
        &format!("/api/v1/forums/{board}/topics"),
        Some(json!({"title": title, "html": format!("<p>{text}</p>"), "source": {}})),
        cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    r.body["topic_id"].as_i64().unwrap()
}

async fn search(app: &Router, cookies: &[String], query: &str) -> Value {
    let r = call(
        app,
        "GET",
        &format!("/api/v1/search?{query}"),
        None,
        cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{query}: {:?}", r.body);
    r.body
}

fn topic_ids(res: &Value, key: &str) -> Vec<i64> {
    res[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["topic_id"].as_i64().unwrap())
        .collect()
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn search_respects_visibility_and_highlights(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    forum(&db, "open", 0).await;
    forum(&db, "vip", 3).await;
    let (_, a) = member(&app, &db, 1).await;
    let (_, v) = member(&app, &db, 3).await;
    let open = topic(
        &app,
        &a,
        "open",
        "出售 Netflix 会员",
        "自动发货的 Netflix 会员，<b>可担保</b>",
    )
    .await;
    let hidden = topic(&app, &a, "open", "隐藏的 Netflix", "Netflix 隐藏").await;
    let vip = topic(&app, &v, "vip", "会员区 Netflix", "Netflix 只给 3 级").await;
    sqlx::query("UPDATE topics SET flags = 8 WHERE id = $1")
        .bind(hidden)
        .execute(&db)
        .await
        .unwrap();
    // 被管理人员隐藏的楼层（status 3）同样搜不到，连作者本人也一样
    let hidden_post = topic(&app, &a, "open", "另一个帖子", "Netflix 被隐藏的楼层").await;
    sqlx::query("UPDATE posts SET status = 3 WHERE topic_id = $1")
        .bind(hidden_post)
        .execute(&db)
        .await
        .unwrap();

    // 游客：看不到隐藏主题与 3 级板块
    let q = format!("q={}", enc("netflix"));
    let res = search(&app, &[], &q).await;
    assert_eq!(topic_ids(&res, "posts"), [open]);
    let hit = &res["posts"][0];
    assert_eq!(hit["topic_title"], "出售 Netflix 会员");
    assert_eq!(
        (hit["floor"].as_i64(), hit["page"].as_i64()),
        (Some(1), Some(1))
    );
    let snippet = hit["snippet"].as_array().unwrap();
    let hits: Vec<&str> = snippet
        .iter()
        .filter(|s| s["hit"] == true)
        .map(|s| s["text"].as_str().unwrap())
        .collect();
    assert_eq!(hits, ["Netflix"]);
    // 正文是过滤后的纯文本：HTML 标签不会出现在片段里
    let text: String = snippet
        .iter()
        .map(|s| s["text"].as_str().unwrap())
        .collect();
    assert_eq!(text, "自动发货的 Netflix 会员，可担保");
    assert_eq!(res["users"].as_array().unwrap().len(), 1);
    assert_eq!(res["forums"][0]["slug"], "open");

    // 3 级用户能搜到会员区；指定板块只搜那一个
    let res = search(&app, &v, &q).await;
    assert_eq!(topic_ids(&res, "posts"), [vip, open]);
    assert_eq!(topic_ids(&search(&app, &a, &q).await, "posts"), [open]);
    let res = search(&app, &v, &format!("{q}&board=vip")).await;
    assert_eq!(topic_ids(&res, "posts"), [vip]);
    // 看不到的板块当作不存在
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/search?{q}&board=vip"),
        None,
        &a,
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);

    // 删除的楼层搜不到
    let first: i64 = sqlx::query_scalar("SELECT id FROM posts WHERE topic_id = $1 AND floor = 1")
        .bind(open)
        .fetch_one(&db)
        .await
        .unwrap();
    let r = call(&app, "DELETE", &format!("/api/v1/posts/{first}"), None, &a).await;
    assert!(r.status.is_success(), "{:?}", r.body);
    assert!(topic_ids(&search(&app, &[], &q).await, "posts").is_empty());

    // 标题搜索带高亮；主题不受楼层删除影响
    let res = search(&app, &[], &format!("{q}&type=topic")).await;
    let topics = res["topics"].as_array().unwrap();
    assert_eq!(topics.len(), 1);
    assert_eq!(topics[0]["topic"]["id"], open);
    assert_eq!(
        topics[0]["title"][1],
        json!({"text": "Netflix", "hit": true})
    );

    // 空查询是参数错误
    let r = call(&app, "GET", "/api/v1/search?q=%20", None, &[]).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn users_by_prefix_and_pages_by_cursor(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    forum(&db, "open", 0).await;
    let (id, a) = member(&app, &db, 1).await;
    set_username(&db, id, "搜索达人").await;
    let res = search(&app, &[], &format!("q={}&type=user", enc("搜索"))).await;
    assert_eq!(res["people"][0]["username"], "搜索达人");

    // 1 楼加 44 个回复：按新到旧分三页，游标接上不重不漏
    let t = topic(&app, &a, "open", "翻页", "分页关键词 0").await;
    for i in 1..45 {
        let r = call(
            &app,
            "POST",
            &format!("/api/v1/topics/{t}/posts"),
            Some(json!({"html": format!("<p>分页关键词 {i}</p>"), "source": {}})),
            &a,
        )
        .await;
        assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    }
    let expected: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM posts WHERE topic_id = $1 ORDER BY id DESC")
            .bind(t)
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(expected.len(), 45);
    let mut seen = Vec::new();
    let mut pages = 0;
    let mut cursor: Option<String> = None;
    loop {
        let mut q = format!("q={}", enc("分页关键词"));
        if let Some(c) = &cursor {
            q.push_str(&format!("&cursor={c}"));
        }
        let res = search(&app, &[], &q).await;
        pages += 1;
        seen.extend(
            res["posts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| p["id"].as_i64().unwrap()),
        );
        match res["next_cursor"].as_str() {
            Some(c) => cursor = Some(c.to_owned()),
            None => break,
        }
    }
    assert_eq!(pages, 3);
    assert_eq!(seen, expected);
    let r = call(&app, "GET", "/api/v1/search?q=a&cursor=bad", None, &[]).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn guests_are_rate_limited_per_ip(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    // 同一个路由 = 同一个客户端 IP（common::router_of）
    for _ in 0..forum_api::routes::search::GUEST_SEARCH_LIMIT {
        let r = call(&app, "GET", "/api/v1/search?q=x", None, &[]).await;
        assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    }
    let r = call(&app, "GET", "/api/v1/search?q=x", None, &[]).await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
}
