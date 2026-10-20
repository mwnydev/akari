//! 论坛核心全流程：板块、主题、回帖、楼层分页、版本化缓存、管理操作、新用户限制、编辑历史与删除。
//! 需要 PostgreSQL（`DATABASE_URL`）与 Valkey（`VALKEY_URL`，未设置则跳过）。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

mod common;

use axum::Router;
use axum::http::StatusCode;
use common::{age, build, call, user};
use forum_db::PgPool;
use forum_db::forums::{self, NewForum};
use forum_db::staff;
use serde_json::{Value, json};

async fn forum(db: &PgPool, slug: &str, view: i16, post: i16) -> i32 {
    forums::create(
        db,
        &NewForum {
            parent_id: None,
            slug,
            name: slug,
            description: "",
            position: 0,
            view_min_level: view,
            post_min_level: post,
            reply_min_level: 1,
            allow_trade: false,
            allow_auction: false,
            color: None,
        },
    )
    .await
    .unwrap()
}

fn doc(text: &str) -> Value {
    json!({"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": text}]}]})
}

async fn post_topic(
    app: &Router,
    cookies: &[String],
    slug: &str,
    title: &str,
    html: &str,
) -> common::Res {
    call(
        app,
        "POST",
        &format!("/api/v1/forums/{slug}/topics"),
        Some(json!({"title": title, "html": html, "source": doc(title)})),
        cookies,
    )
    .await
}

async fn reply(app: &Router, cookies: &[String], topic: i64, html: &str) -> common::Res {
    call(
        app,
        "POST",
        &format!("/api/v1/topics/{topic}/posts"),
        Some(json!({"html": html, "source": doc("r")})),
        cookies,
    )
    .await
}

/// 已解除新用户限制的登录用户。
async fn member(app: &Router, db: &PgPool) -> (i64, Vec<String>) {
    let (id, cookies) = user(app, db).await;
    age(db, id).await;
    (id, cookies)
}

fn ids(list: &Value) -> Vec<i64> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_i64().unwrap())
        .collect()
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn topic_reply_and_board_order(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    forum(&db, "general", 0, 1).await;
    let (alice, a) = member(&app, &db).await;
    let (_bob, b) = member(&app, &db).await;

    // 游客不能发帖
    let r = post_topic(&app, &[], "general", "x", "<p>x</p>").await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);

    let r = post_topic(
        &app,
        &a,
        "general",
        "  第一个主题 ",
        "<p>你好<script>x</script></p>",
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    let t1 = r.body["topic_id"].as_i64().unwrap();
    let t2 = post_topic(&app, &a, "general", "第二个", "<p>2</p>")
        .await
        .body["topic_id"]
        .as_i64()
        .unwrap();

    // 新的在前
    let r = call(&app, "GET", "/api/v1/forums/general/topics", None, &[]).await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert_eq!(ids(&r.body["topics"]), vec![t2, t1]);
    assert_eq!(r.body["topics"][1]["title"], "第一个主题");
    assert!(
        r.body["users"]
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["id"] == alice)
    );

    // 回帖把旧主题顶上来，并给楼主发通知
    let r = reply(&app, &b, t1, "<p>沙发</p>").await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    assert_eq!(
        (r.body["floor"].as_i64(), r.body["page"].as_i64()),
        (Some(2), Some(1))
    );
    let r = call(&app, "GET", "/api/v1/forums/general/topics", None, &[]).await;
    assert_eq!(ids(&r.body["topics"]), vec![t1, t2]);
    assert_eq!(r.body["topics"][0]["replies"], 1);
    let notes: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox WHERE kind = 'notify.reply'")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(notes, 1);

    // 主题页：HTML 已过滤；楼主自己回帖不通知
    let r = call(&app, "GET", &format!("/api/v1/topics/{t1}"), None, &[]).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.body["posts"][0]["html"], "<p>你好</p>");
    assert_eq!(r.body["posts"].as_array().unwrap().len(), 2);
    assert_eq!(
        (
            r.body["total_pages"].as_i64(),
            r.body["can_reply"].as_bool()
        ),
        (Some(1), Some(false))
    );
    reply(&app, &a, t1, "<p>自己顶</p>").await;
    let notes: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox WHERE kind = 'notify.reply'")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(notes, 1);

    // 不存在的板块与主题
    assert_eq!(
        call(&app, "GET", "/api/v1/forums/nope/topics", None, &[])
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&app, "GET", "/api/v1/topics/999999", None, &[])
            .await
            .status,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn page1_cache_is_invalidated_by_reply_and_edit(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    forum(&db, "general", 0, 1).await;
    let (_, a) = member(&app, &db).await;
    let r = post_topic(&app, &a, "general", "缓存", "<p>v1</p>").await;
    let (topic, post) = (
        r.body["topic_id"].as_i64().unwrap(),
        r.body["post_id"].as_i64().unwrap(),
    );
    let path = format!("/api/v1/topics/{topic}");
    let get = || call(&app, "GET", &path, None, &[]);
    assert_eq!(get().await.body["posts"].as_array().unwrap().len(), 1); // 写入缓存

    reply(&app, &a, topic, "<p>r</p>").await;
    assert_eq!(get().await.body["posts"].as_array().unwrap().len(), 2);

    let r = call(
        &app,
        "PATCH",
        &format!("/api/v1/posts/{post}"),
        Some(json!({"html": "<p>v2</p>", "source": doc("v2"), "title": "新标题"})),
        &a,
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{:?}", r.body);
    let body = get().await.body;
    assert_eq!(body["posts"][0]["html"], "<p>v2</p>");
    assert_eq!(body["topic"]["title"], "新标题");
    assert!(body["posts"][0]["edited_at"].is_string());
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn floor_pages(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let f = forum(&db, "general", 0, 1).await;
    let (uid, a) = member(&app, &db).await;
    let topic = post_topic(&app, &a, "general", "长帖", "<p>1</p>")
        .await
        .body["topic_id"]
        .as_i64()
        .unwrap();
    // 直接灌到 45 楼（不经限流）
    sqlx::query(
        "INSERT INTO posts (topic_id, floor, author_id, html, plain_text)
         SELECT $1, g, $2, '<p>' || g || '</p>', g::text FROM generate_series(2, 45) g",
    )
    .bind(topic)
    .bind(uid)
    .execute(&db)
    .await
    .unwrap();
    sqlx::query("UPDATE topics SET last_floor = 45 WHERE id = $1 AND forum_id = $2")
        .bind(topic)
        .bind(f)
        .execute(&db)
        .await
        .unwrap();
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/topics/{topic}?page=3"),
        None,
        &[],
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    let floors: Vec<i64> = r.body["posts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["floor"].as_i64().unwrap())
        .collect();
    assert_eq!(floors, (41..=45).collect::<Vec<_>>());
    assert_eq!(r.body["total_pages"], 3);
    // 新回复是 46 楼，在第 3 页
    let r = reply(&app, &a, topic, "<p>46</p>").await;
    assert_eq!(
        (r.body["floor"].as_i64(), r.body["page"].as_i64()),
        (Some(46), Some(3))
    );
    assert_eq!(
        call(
            &app,
            "GET",
            &format!("/api/v1/topics/{topic}?page=0"),
            None,
            &[]
        )
        .await
        .status,
        StatusCode::BAD_REQUEST
    );
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn board_pages_then_cursor(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let f = forum(&db, "big", 0, 1).await;
    // 10060 个普通主题 + 1 个置顶 + 1 个隐藏：有序集合存满 1 万个，之后走游标
    sqlx::query(
        "INSERT INTO topics (forum_id, author_id, title, last_post_at)
         SELECT $1, 1, 't' || g, now() - (g || ' seconds')::interval FROM generate_series(1, 10060) g",
    )
    .bind(f)
    .execute(&db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO topics (forum_id, author_id, title, flags, last_post_at)
         VALUES ($1, 1, 'pinned', 1, now() - interval '1 day'), ($1, 1, 'hidden', 8, now())",
    )
    .bind(f)
    .execute(&db)
    .await
    .unwrap();
    let newest: i64 = sqlx::query_scalar("SELECT id FROM topics WHERE title = 't1'")
        .fetch_one(&db)
        .await
        .unwrap();

    let r = call(&app, "GET", "/api/v1/forums/big/topics", None, &[]).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.body["topics"].as_array().unwrap().len(), 50);
    assert_eq!(r.body["topics"][0]["id"], newest, "hidden topic excluded");
    assert_eq!(r.body["pinned"][0]["title"], "pinned");
    assert_eq!(r.body["next_page"], 2);

    let r = call(&app, "GET", "/api/v1/forums/big/topics?page=2", None, &[]).await;
    assert!(r.body["pinned"].as_array().unwrap().is_empty());
    assert_eq!(r.body["topics"][0]["title"], "t51");

    let r = call(&app, "GET", "/api/v1/forums/big/topics?page=200", None, &[]).await;
    assert_eq!(r.body["topics"][49]["title"], "t10000");
    assert!(r.body["next_page"].is_null());
    let cursor = r.body["next_cursor"].as_str().unwrap().to_owned();

    let r = call(
        &app,
        "GET",
        &format!("/api/v1/forums/big/topics?cursor={cursor}"),
        None,
        &[],
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert_eq!(r.body["topics"][0]["title"], "t10001");
    let cursor = r.body["next_cursor"].as_str().unwrap().to_owned();
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/forums/big/topics?cursor={cursor}"),
        None,
        &[],
    )
    .await;
    assert_eq!(r.body["topics"].as_array().unwrap().len(), 10);
    assert!(r.body["next_cursor"].is_null());

    for bad in ["?page=0", "?page=201", "?cursor=zzz", "?page=1&cursor=1_1"] {
        let r = call(
            &app,
            "GET",
            &format!("/api/v1/forums/big/topics{bad}"),
            None,
            &[],
        )
        .await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{bad}");
    }
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn moderation_flags_move_and_audit(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let general = forum(&db, "general", 0, 1).await;
    let other = forum(&db, "other", 0, 1).await;
    let (_, a) = member(&app, &db).await;
    let (mod_id, m) = member(&app, &db).await;
    staff::grant(&db, mod_id, staff::ROLE_MODERATOR, Some(general), None)
        .await
        .unwrap();
    let topic = post_topic(&app, &a, "general", "管理", "<p>x</p>")
        .await
        .body["topic_id"]
        .as_i64()
        .unwrap();
    let act = |cookies: Vec<String>, action: &'static str, forum_id: Option<i32>| {
        let app = app.clone();
        async move {
            call(
                &app,
                "POST",
                &format!("/api/v1/topics/{topic}/moderation"),
                Some(json!({"action": action, "reason": "测试", "forum_id": forum_id})),
                &cookies,
            )
            .await
        }
    };
    let board = || call(&app, "GET", "/api/v1/forums/general/topics", None, &[]);

    // 普通用户不能管理
    assert_eq!(
        act(a.clone(), "pin", None).await.status,
        StatusCode::FORBIDDEN
    );
    // 理由必填
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/topics/{topic}/moderation"),
        Some(json!({"action": "pin", "reason": " "})),
        &m,
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);

    board().await; // 建好有序集合
    let r = act(m.clone(), "pin", None).await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert_eq!(r.body["flags"]["pinned"], true);
    let b = board().await.body;
    assert_eq!(ids(&b["pinned"]), vec![topic]);
    assert!(ids(&b["topics"]).is_empty());

    act(m.clone(), "unpin", None).await;
    let b = board().await.body;
    assert!(ids(&b["pinned"]).is_empty());
    assert_eq!(ids(&b["topics"]), vec![topic]);

    // 锁定：普通用户不能回复，版主可以
    act(m.clone(), "lock", None).await;
    let r = reply(&app, &a, topic, "<p>x</p>").await;
    assert_eq!(
        (r.status, r.body["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("TOPIC_LOCKED"))
    );
    assert_eq!(
        reply(&app, &m, topic, "<p>版主回复</p>").await.status,
        StatusCode::CREATED
    );
    act(m.clone(), "unlock", None).await;

    // 隐藏：对普通用户不存在，版主可见
    act(m.clone(), "hide", None).await;
    assert!(ids(&board().await.body["topics"]).is_empty());
    let path = format!("/api/v1/topics/{topic}");
    assert_eq!(
        call(&app, "GET", &path, None, &a).await.status,
        StatusCode::NOT_FOUND
    );
    let r = call(&app, "GET", &path, None, &m).await;
    assert_eq!(
        (r.status, r.body["can_moderate"].as_bool()),
        (StatusCode::OK, Some(true))
    );
    act(m.clone(), "unhide", None).await;

    // 移动：版主不管目标板块 → 403；管理员可以
    assert_eq!(
        act(m.clone(), "move", Some(other)).await.status,
        StatusCode::FORBIDDEN
    );
    let (admin_id, adm) = member(&app, &db).await;
    staff::grant(&db, admin_id, staff::ROLE_ADMIN, None, None)
        .await
        .unwrap();
    let r = act(adm.clone(), "move", Some(other)).await;
    assert_eq!(
        (r.status, r.body["forum_id"].as_i64()),
        (StatusCode::OK, Some(i64::from(other)))
    );
    assert!(ids(&board().await.body["topics"]).is_empty());
    let b = call(&app, "GET", "/api/v1/forums/other/topics", None, &[])
        .await
        .body;
    assert_eq!(ids(&b["topics"]), vec![topic]);
    assert_eq!(act(adm, "move", None).await.status, StatusCode::BAD_REQUEST);

    let logs: Vec<String> = sqlx::query_scalar("SELECT action FROM admin_audit_logs ORDER BY id")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(
        logs,
        [
            "topic.pin",
            "topic.unpin",
            "topic.lock",
            "topic.unlock",
            "topic.hide",
            "topic.unhide",
            "topic.move"
        ]
    );
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn new_user_limits(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    forum(&db, "general", 0, 1).await;
    let (_, fresh) = user(&app, &db).await; // 注册不满 24 小时

    let r = post_topic(
        &app,
        &fresh,
        "general",
        "外链",
        r#"<p><a href="https://evil.test/">x</a></p>"#,
    )
    .await;
    assert_eq!(
        (r.status, r.body["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("EXTERNAL_LINK_FORBIDDEN"))
    );
    // 站内链接可以
    let r = post_topic(
        &app,
        &fresh,
        "general",
        "站内",
        r#"<p><a href="https://forum.test/t/1">x</a></p>"#,
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    let topic = r.body["topic_id"].as_i64().unwrap();
    // 每小时合计 5 条（主题 + 回帖）
    for _ in 0..4 {
        assert_eq!(
            reply(&app, &fresh, topic, "<p>r</p>").await.status,
            StatusCode::CREATED
        );
    }
    let r = reply(&app, &fresh, topic, "<p>r</p>").await;
    assert_eq!(
        (r.status, r.body["code"].as_str()),
        (StatusCode::TOO_MANY_REQUESTS, Some("RATE_LIMITED"))
    );

    // 内容校验在限流之前：空内容、标题过长都是 400
    let (_, a) = member(&app, &db).await;
    let r = post_topic(&app, &a, "general", "空", "<p> </p>").await;
    assert_eq!(r.body["code"], "CONTENT_EMPTY");
    let r = post_topic(&app, &a, "general", &"长".repeat(121), "<p>x</p>").await;
    assert_eq!(r.body["code"], "INVALID_TITLE");
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn new_user_may_post_own_uploaded_images(db: PgPool) {
    let Some(mut state) =
        common::state_with(db.clone(), common::wallet_state(), &common::namespace())
    else {
        return;
    };
    // 只用到公开地址（判断本站图片），不连存储桶
    let storage = forum_media::Storage::new(&forum_media::StorageConfig {
        endpoint: "http://127.0.0.1:9".to_owned(),
        bucket: "b".to_owned(),
        region: "auto".to_owned(),
        access_key: "k".to_owned(),
        secret_key: "s".to_owned(),
        public_base: "https://img.forum.test/".to_owned(),
    })
    .unwrap();
    state.media = Some(std::sync::Arc::new(storage));
    let app = common::router_of(state);
    forum(&db, "general", 0, 1).await;
    let (_, fresh) = user(&app, &db).await; // 注册不满 24 小时

    let r = post_topic(
        &app,
        &fresh,
        "general",
        "晒图",
        r#"<p>新键盘<img src="https://img.forum.test/i/a.webp"></p>"#,
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    let html: String = sqlx::query_scalar("SELECT html FROM posts WHERE topic_id = $1")
        .bind(r.body["topic_id"].as_i64().unwrap())
        .fetch_one(&db)
        .await
        .unwrap();
    assert!(html.contains("https://img.forum.test/i/a.webp"), "{html}");
    // 别处的图片照旧被过滤，外链照旧拒绝
    let r = post_topic(
        &app,
        &fresh,
        "general",
        "外链",
        r#"<p>x <a href="https://img.forum.test.evil.test/">y</a></p>"#,
    )
    .await;
    assert_eq!(r.body["code"], "EXTERNAL_LINK_FORBIDDEN");
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn edit_history_and_delete(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let f = forum(&db, "general", 0, 1).await;
    let (_, a) = member(&app, &db).await;
    let (_, b) = member(&app, &db).await;
    let (mod_id, m) = member(&app, &db).await;
    staff::grant(&db, mod_id, staff::ROLE_MODERATOR, Some(f), None)
        .await
        .unwrap();
    let topic = post_topic(&app, &a, "general", "原标题", "<p>v1</p>")
        .await
        .body["topic_id"]
        .as_i64()
        .unwrap();
    let r2 = reply(&app, &b, topic, "<p>b1</p>").await.body["post_id"]
        .as_i64()
        .unwrap();
    let edit = |cookies: Vec<String>, post: i64, body: Value| {
        let app = app.clone();
        async move {
            call(
                &app,
                "PATCH",
                &format!("/api/v1/posts/{post}"),
                Some(body),
                &cookies,
            )
            .await
        }
    };

    // 别人不能改；非 1 楼不能改标题
    assert_eq!(
        edit(
            a.clone(),
            r2,
            json!({"html": "<p>x</p>", "source": doc("x")})
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        edit(
            b.clone(),
            r2,
            json!({"html": "<p>x</p>", "source": doc("x"), "title": "t"})
        )
        .await
        .status,
        StatusCode::BAD_REQUEST
    );
    // 作者编辑两次 → 两条历史，新的在前
    for v in ["b2", "b3"] {
        let r = edit(
            b.clone(),
            r2,
            json!({"html": format!("<p>{v}</p>"), "source": doc(v)}),
        )
        .await;
        assert_eq!(r.status, StatusCode::NO_CONTENT, "{:?}", r.body);
    }
    let h = call(
        &app,
        "GET",
        &format!("/api/v1/posts/{r2}/history"),
        None,
        &[],
    )
    .await;
    let olds: Vec<&str> = h.body["edits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["html"].as_str().unwrap())
        .collect();
    assert_eq!(olds, ["<p>b2</p>", "<p>b1</p>"]);
    // 原稿只有作者和管理人员能取
    let src = format!("/api/v1/posts/{r2}/source");
    assert_eq!(
        call(&app, "GET", &src, None, &b).await.body["source"],
        doc("b3")
    );
    assert_eq!(
        call(&app, "GET", &src, None, &a).await.status,
        StatusCode::FORBIDDEN
    );

    // 版主不能编辑他人楼层（ADR 0013）；管理员可以，写审计
    assert_eq!(
        edit(
            m.clone(),
            r2,
            json!({"html": "<p>mod</p>", "source": doc("m")})
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    let (admin_id, adm) = member(&app, &db).await;
    staff::grant(&db, admin_id, staff::ROLE_ADMIN, None, None)
        .await
        .unwrap();
    assert_eq!(
        call(&app, "GET", &src, None, &adm).await.body["source"],
        doc("b3")
    );
    assert_eq!(
        call(&app, "GET", &src, None, &m).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        edit(
            adm.clone(),
            r2,
            json!({"html": "<p>admin</p>", "source": doc("a")})
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );

    // 作者删除：楼层保留、内容清空；之后的回帖楼层号继续递增
    let del = call(&app, "DELETE", &format!("/api/v1/posts/{r2}"), None, &b).await;
    assert_eq!(del.status, StatusCode::NO_CONTENT);
    let page = call(&app, "GET", &format!("/api/v1/topics/{topic}"), None, &[])
        .await
        .body;
    assert_eq!(
        (
            page["posts"][1]["deleted"].as_bool(),
            page["posts"][1]["html"].as_str()
        ),
        (Some(true), Some(""))
    );
    assert_eq!(
        reply(&app, &b, topic, "<p>b again</p>").await.body["floor"],
        3
    );
    // 已删除的不能再编辑；普通访客看不到已删除楼层的历史
    assert_eq!(
        edit(
            b.clone(),
            r2,
            json!({"html": "<p>x</p>", "source": doc("x")})
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &app,
            "GET",
            &format!("/api/v1/posts/{r2}/history"),
            None,
            &[]
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );

    // 版主删除他人楼层必须写理由
    let first: i64 = sqlx::query_scalar("SELECT id FROM posts WHERE topic_id = $1 AND floor = 1")
        .bind(topic)
        .fetch_one(&db)
        .await
        .unwrap();
    let path = format!("/api/v1/posts/{first}");
    assert_eq!(
        call(&app, "DELETE", &path, None, &m).await.status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &app,
            "DELETE",
            &format!("{path}?reason=%E5%B9%BF%E5%91%8A"),
            None,
            &m
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );
    let logs: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT action, reason FROM admin_audit_logs ORDER BY id")
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(
        logs,
        [
            ("post.edit".to_owned(), None),
            ("post.delete".to_owned(), Some("广告".to_owned()))
        ]
    );
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn forum_level_permissions(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    forum(&db, "public", 0, 1).await;
    let vip = forum(&db, "vip", 5, 5).await;
    forum(&db, "lv3", 0, 3).await;
    let (_, a) = member(&app, &db).await;

    let names = |body: Value| -> Vec<String> {
        body.as_array()
            .unwrap()
            .iter()
            .map(|f| f["slug"].as_str().unwrap().to_owned())
            .collect()
    };
    // 游客看不到需要 5 级的板块
    assert_eq!(
        names(call(&app, "GET", "/api/v1/forums", None, &[]).await.body),
        ["public", "lv3"]
    );
    assert_eq!(
        call(&app, "GET", "/api/v1/forums/vip/topics", None, &a)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    let r = post_topic(&app, &a, "lv3", "x", "<p>x</p>").await;
    assert_eq!(
        (r.status, r.body["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("FORBIDDEN"))
    );
    assert_eq!(
        post_topic(&app, &a, "vip", "x", "<p>x</p>").await.status,
        StatusCode::NOT_FOUND
    );

    // 该板块的版主看得到、能发帖
    let (mod_id, m) = member(&app, &db).await;
    staff::grant(&db, mod_id, staff::ROLE_MODERATOR, Some(vip), None)
        .await
        .unwrap();
    assert!(
        names(call(&app, "GET", "/api/v1/forums", None, &m).await.body).contains(&"vip".to_owned())
    );
    assert_eq!(
        post_topic(&app, &m, "vip", "x", "<p>x</p>").await.status,
        StatusCode::CREATED
    );
    assert_eq!(
        call(&app, "GET", "/api/v1/forums/vip/topics", None, &m)
            .await
            .status,
        StatusCode::OK
    );
}

/// 板块色块只能取固定色板里的颜色（迁移里的 CHECK），列表原样返回；没设置的为空。
#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn forum_colors_come_from_the_palette(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let new = |slug, color| NewForum {
        parent_id: None,
        slug,
        name: slug,
        description: "",
        position: 0,
        view_min_level: 0,
        post_min_level: 1,
        reply_min_level: 1,
        allow_trade: false,
        allow_auction: false,
        color,
    };
    forums::create(&db, &new("green", Some("#2e9d5c")))
        .await
        .unwrap();
    forum(&db, "plain", 0, 1).await;
    assert!(
        forums::create(&db, &new("odd", Some("#123456")))
            .await
            .is_err()
    );

    let r = call(&app, "GET", "/api/v1/forums", None, &[]).await;
    let colors: Vec<(&str, &Value)> = r
        .body
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["slug"].as_str().unwrap(), &f["color"]))
        .collect();
    assert_eq!(
        colors,
        [("green", &json!("#2e9d5c")), ("plain", &Value::Null)]
    );
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn openapi_lists_forum_endpoints(db: PgPool) {
    let Some(app) = build(db) else { return };
    let r = call(&app, "GET", "/api/v1/openapi.json", None, &[]).await;
    for path in [
        "/api/v1/forums",
        "/api/v1/forums/{slug}/topics",
        "/api/v1/topics/{id}",
        "/api/v1/topics/{id}/posts",
        "/api/v1/topics/{id}/moderation",
        "/api/v1/posts/{id}",
        "/api/v1/posts/{id}/history",
        "/api/v1/posts/{id}/source",
    ] {
        assert!(r.body["paths"].get(path).is_some(), "missing {path}");
    }
    for schema in [
        "BoardResponse",
        "TopicPage",
        "ModerationAction",
        "ErrorCode",
    ] {
        assert!(
            r.body["components"]["schemas"].get(schema).is_some(),
            "missing {schema}"
        );
    }
}
