//! 用户名、个人主页、用户的帖子、@提及搜索、名片缓存、跨板块最新主题、精华经验、图片上传接口。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

mod common;

use axum::Router;
use axum::http::StatusCode;
use common::{age, build, call, user};
use forum_db::forums::{self, NewForum};
use forum_db::{PgPool, staff};
use serde_json::{Value, json};

async fn forum(db: &PgPool, slug: &str, view: i16) -> i32 {
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
    .unwrap()
}

async fn member(app: &Router, db: &PgPool) -> (i64, Vec<String>) {
    let (id, cookies) = user(app, db).await;
    age(db, id).await;
    (id, cookies)
}

async fn post_topic(app: &Router, cookies: &[String], slug: &str, title: &str) -> i64 {
    let r = call(
        app,
        "POST",
        &format!("/api/v1/forums/{slug}/topics"),
        Some(json!({"title": title, "html": format!("<p>{title} body</p>"), "source": {"type": "doc"}})),
        cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    r.body["topic_id"].as_i64().unwrap()
}

async fn set_name(app: &Router, cookies: &[String], name: &str) -> common::Res {
    call(
        app,
        "PUT",
        "/api/v1/me/username",
        Some(json!({"username": name})),
        cookies,
    )
    .await
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn username_rules_and_profile(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let (a, ca) = member(&app, &db).await;
    let (_, cb) = member(&app, &db).await;

    for bad in ["x", "12345", "admin", "管理员", "a b", "аlice"] {
        let r = set_name(&app, &ca, bad).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{bad}");
        assert_eq!(r.body["code"], "INVALID_USERNAME");
    }
    let r = set_name(&app, &ca, " 小明_Lam ").await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert_eq!(r.body["username"], "小明_Lam");
    let r = set_name(&app, &cb, "小明_lam").await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    assert_eq!(r.body["code"], "USERNAME_TAKEN");
    let r = set_name(&app, &ca, "小明2").await;
    assert_eq!(r.body["code"], "USERNAME_CHANGE_TOO_SOON");
    assert_eq!(
        call(
            &app,
            "PUT",
            "/api/v1/me/username",
            Some(json!({"username": "zz"})),
            &[]
        )
        .await
        .status,
        StatusCode::UNAUTHORIZED
    );

    // 主页：按用户名（不区分大小写）或 ID
    forum(&db, "general", 0).await;
    post_topic(&app, &ca, "general", "hello").await;
    for path in [
        "/api/v1/users/%E5%B0%8F%E6%98%8E_LAM",
        &format!("/api/v1/users/{a}"),
    ] {
        let r = call(&app, "GET", path, None, &[]).await;
        assert_eq!(r.status, StatusCode::OK, "{path}: {:?}", r.body);
        assert_eq!(r.body["id"], a);
        assert_eq!(r.body["username"], "小明_Lam");
        assert_eq!(r.body["level"], 1);
        assert_eq!(r.body["topics"], 1);
        assert_eq!(r.body["posts"], 1);
        assert_eq!(r.body["trade"]["success"], 0);
        assert_eq!(r.body["trade"]["success_rate_bps"], Value::Null);
    }
    assert_eq!(
        call(&app, "GET", "/api/v1/users/nobody", None, &[])
            .await
            .status,
        StatusCode::NOT_FOUND
    );

    // 隐身：最近活跃只到天
    sqlx::query(
        "UPDATE users SET last_seen_at = '2026-10-18T13:14:15Z', show_online = false WHERE id = $1",
    )
    .bind(a)
    .execute(&db)
    .await
    .unwrap();
    let r = call(&app, "GET", &format!("/api/v1/users/{a}"), None, &[]).await;
    assert_eq!(r.body["last_active_at"], "2026-10-18T00:00:00Z");
    assert_eq!(r.body["last_active_day_only"], true);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn user_posts_respect_visibility_and_cursor(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    forum(&db, "general", 0).await;
    forum(&db, "vip", 3).await;
    let (a, ca) = member(&app, &db).await;
    sqlx::query("INSERT INTO user_levels (user_id, level) VALUES ($1, 3)")
        .bind(a)
        .execute(&db)
        .await
        .unwrap();
    // 1 个主题 + 21 条回复 = 22 楼
    let public = post_topic(&app, &ca, "general", "t").await;
    for i in 0..21 {
        let r = call(
            &app,
            "POST",
            &format!("/api/v1/topics/{public}/posts"),
            Some(json!({"html": format!("<p>reply {i}</p>"), "source": {"type": "doc"}})),
            &ca,
        )
        .await;
        assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    }
    let secret = post_topic(&app, &ca, "vip", "secret").await;

    let path = format!("/api/v1/users/{a}/posts");
    // 游客看不到 3 级板块里的帖子
    let r = call(&app, "GET", &path, None, &[]).await;
    assert_eq!(r.status, StatusCode::OK);
    let posts = r.body["posts"].as_array().unwrap();
    assert_eq!(posts.len(), 20);
    assert_eq!(posts[0]["topic_id"], public);
    assert_eq!(posts[0]["floor"], 22);
    assert_eq!(posts[0]["excerpt"], "reply 20");
    assert_eq!(posts[0]["page"], 2);
    assert_eq!(posts[0]["topic_title"], "t");
    let cursor = r.body["next_cursor"].as_i64().unwrap();
    let r = call(&app, "GET", &format!("{path}?cursor={cursor}"), None, &[]).await;
    assert_eq!(r.body["posts"].as_array().unwrap().len(), 2);
    assert_eq!(r.body["next_cursor"], Value::Null);
    // 自己（3 级）能看到
    let r = call(&app, "GET", &path, None, &ca).await;
    assert_eq!(r.body["posts"][0]["topic_id"], secret);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn mention_search_and_cards(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let (a, ca) = member(&app, &db).await;
    let (b, cb) = member(&app, &db).await;
    set_name(&app, &ca, "lambda").await;
    set_name(&app, &cb, "Lamport").await;

    let r = call(&app, "GET", "/api/v1/users/search?q=LAM", None, &[]).await;
    assert_eq!(r.status, StatusCode::OK);
    let names: Vec<&str> = r
        .body
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["username"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["lambda", "Lamport"]);
    for q in ["", "%25", "l%20a", "zzz"] {
        let r = call(
            &app,
            "GET",
            &format!("/api/v1/users/search?q={q}"),
            None,
            &[],
        )
        .await;
        assert_eq!(r.body, json!([]), "{q}");
    }

    let cards = |ids: String| {
        let app = app.clone();
        async move {
            call(
                &app,
                "GET",
                &format!("/api/v1/users/cards?ids={ids}"),
                None,
                &[],
            )
            .await
        }
    };
    let r = cards(format!("{b},{a},{b},999999")).await;
    assert_eq!(r.status, StatusCode::OK);
    let got = r.body.as_array().unwrap();
    assert_eq!(got.len(), 2, "duplicates and unknown ids dropped");
    assert_eq!(got[0]["id"], b);
    assert_eq!(got[0]["username"], "Lamport");
    assert_eq!(got[0]["reputation"]["success"], 0);
    assert_eq!(cards("x".into()).await.status, StatusCode::BAD_REQUEST);
    let many: Vec<String> = (1..=101).map(|i| i.to_string()).collect();
    assert_eq!(cards(many.join(",")).await.status, StatusCode::BAD_REQUEST);

    // 改名后名片立即更新（缓存已失效）
    assert_eq!(cards(a.to_string()).await.body[0]["username"], "lambda");
    sqlx::query("UPDATE users SET username_changed_at = NULL WHERE id = $1")
        .bind(a)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(set_name(&app, &ca, "lambda2").await.status, StatusCode::OK);
    assert_eq!(cards(a.to_string()).await.body[0]["username"], "lambda2");
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn featured_exp_reaches_card(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    forum(&db, "general", 0).await;
    let (a, ca) = member(&app, &db).await;
    let (admin, cadmin) = member(&app, &db).await;
    staff::grant(&db, admin, staff::ROLE_ADMIN, None, None)
        .await
        .unwrap();
    let topic = post_topic(&app, &ca, "general", "good").await;
    sqlx::query("INSERT INTO user_levels (user_id, exp) VALUES ($1, 60) ON CONFLICT (user_id) DO UPDATE SET exp = 60")
        .bind(a)
        .execute(&db)
        .await
        .unwrap();
    let card = || {
        let app = app.clone();
        async move {
            call(
                &app,
                "GET",
                &format!("/api/v1/users/cards?ids={a}"),
                None,
                &[],
            )
            .await
            .body[0]
                .clone()
        }
    };
    assert_eq!(card().await["level"], 1);
    let moderate = |action: &'static str| {
        let (app, cadmin) = (app.clone(), cadmin.clone());
        async move {
            call(
                &app,
                "POST",
                &format!("/api/v1/topics/{topic}/moderation"),
                Some(json!({"action": action, "reason": "精华"})),
                &cadmin,
            )
            .await
        }
    };
    assert_eq!(moderate("feature").await.status, StatusCode::OK);
    // 60 + 50 跨过 100：升到 2 级，名片缓存随之失效
    assert_eq!(card().await["level"], 2);
    let r = call(&app, "GET", &format!("/api/v1/users/{a}"), None, &[]).await;
    assert_eq!(r.body["exp"], 110);
    assert_eq!(moderate("unfeature").await.status, StatusCode::OK);
    let r = call(&app, "GET", &format!("/api/v1/users/{a}"), None, &[]).await;
    assert_eq!(r.body["exp"], 60);
    assert_eq!(r.body["level"], 2, "level never drops");
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn latest_across_visible_boards(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    forum(&db, "general", 0).await;
    forum(&db, "tech", 0).await;
    forum(&db, "vip", 3).await;
    let (a, ca) = member(&app, &db).await;
    sqlx::query("INSERT INTO user_levels (user_id, level) VALUES ($1, 3)")
        .bind(a)
        .execute(&db)
        .await
        .unwrap();
    let t1 = post_topic(&app, &ca, "general", "g1").await;
    let t2 = post_topic(&app, &ca, "tech", "t1").await;
    let t3 = post_topic(&app, &ca, "vip", "v1").await;
    let ids = |r: &common::Res| -> Vec<i64> {
        r.body["topics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_i64().unwrap())
            .collect()
    };
    let r = call(&app, "GET", "/api/v1/topics/latest", None, &[]).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(ids(&r), vec![t2, t1], "guest: vip board hidden");
    assert_eq!(r.body["next_cursor"], Value::Null);
    let slugs: Vec<&str> = r.body["forums"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["slug"].as_str().unwrap())
        .collect();
    assert_eq!(slugs.len(), 2);
    assert_eq!(r.body["users"][0]["id"], a);
    let r = call(&app, "GET", "/api/v1/topics/latest", None, &ca).await;
    assert_eq!(ids(&r), vec![t3, t2, t1]);

    // 回复把主题顶到最前；隐藏的主题移出
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/topics/{t1}/posts"),
        Some(json!({"html": "<p>up</p>", "source": {"type": "doc"}})),
        &ca,
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED);
    let (admin, cadmin) = member(&app, &db).await;
    staff::grant(&db, admin, staff::ROLE_ADMIN, None, None)
        .await
        .unwrap();
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/topics/{t2}/moderation"),
        Some(json!({"action": "hide", "reason": "spam"})),
        &cadmin,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    let r = call(&app, "GET", "/api/v1/topics/latest", None, &[]).await;
    assert_eq!(ids(&r), vec![t1]);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn latest_continues_past_the_sorted_set(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let f = forum(&db, "big", 0).await;
    sqlx::query(
        "INSERT INTO topics (forum_id, author_id, title, last_post_at)
         SELECT $1, 1, 't' || g, now() - (g || ' seconds')::interval FROM generate_series(1, 10030) g",
    )
    .bind(f)
    .execute(&db)
    .await
    .unwrap();
    let r = call(&app, "GET", "/api/v1/topics/latest", None, &[]).await;
    assert_eq!(r.body["topics"][0]["title"], "t1");
    assert_eq!(r.body["topics"].as_array().unwrap().len(), 50);
    // 有序集合里第 9990 新的主题之后：先取有序集合剩下的 10 个，再从数据库接着取
    let (at, id): (time::OffsetDateTime, i64) =
        sqlx::query_as("SELECT last_post_at, id FROM topics WHERE title = 't9990'")
            .fetch_one(&db)
            .await
            .unwrap();
    let cursor = format!("{}_{id}", at.unix_timestamp_nanos() / 1000);
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/topics/latest?cursor={cursor}"),
        None,
        &[],
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    let titles: Vec<&str> = r.body["topics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["title"].as_str().unwrap())
        .collect();
    let expect: Vec<String> = (9991..=10030).map(|g| format!("t{g}")).collect();
    assert_eq!(titles, expect);
    assert_eq!(r.body["next_cursor"], Value::Null);
    assert_eq!(
        call(&app, "GET", "/api/v1/topics/latest?cursor=bad", None, &[])
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn uploads_presign_complete_and_status(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let (_, ca) = member(&app, &db).await;
    let (_, cb) = member(&app, &db).await;
    let new = |body: Value| {
        let (app, ca) = (app.clone(), ca.clone());
        async move { call(&app, "POST", "/api/v1/uploads", Some(body), &ca).await }
    };
    let Some(storage) = forum_testkit::require_s3(common::media()) else {
        let r = new(json!({"content_type": "image/jpeg", "size": 1000})).await;
        assert_eq!(r.status, StatusCode::SERVICE_UNAVAILABLE);
        return;
    };
    storage.ensure_bucket().await.unwrap();
    let r = new(json!({"content_type": "image/svg+xml", "size": 1000})).await;
    assert_eq!(r.body["code"], "UNSUPPORTED_IMAGE");
    let r = new(json!({"content_type": "image/png", "size": 10 * 1024 * 1024 + 1})).await;
    assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE);

    let body = b"\x89PNG\r\n\x1a\nnot really".to_vec();
    let r = new(json!({"content_type": "image/png", "size": body.len()})).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    let id = r.body["id"].as_i64().unwrap();
    assert_eq!(r.body["headers"]["content-type"], "image/png");
    let url = r.body["upload_url"].as_str().unwrap();
    assert!(url.contains("/tmp/"), "{url}");
    let put = reqwest::Client::new()
        .put(url)
        .header("content-type", "image/png")
        .body(body)
        .send()
        .await
        .unwrap();
    assert!(put.status().is_success(), "{}", put.status());

    let path = format!("/api/v1/uploads/{id}");
    assert_eq!(
        call(&app, "GET", &path, None, &ca).await.body["status"],
        "pending"
    );
    // 别人的上传当作不存在
    assert_eq!(
        call(&app, "POST", &format!("{path}/complete"), None, &cb)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    let r = call(&app, "POST", &format!("{path}/complete"), None, &ca).await;
    assert_eq!(r.status, StatusCode::ACCEPTED);
    assert_eq!(r.body["status"], "processing");
    let again = call(&app, "POST", &format!("{path}/complete"), None, &ca).await;
    assert_eq!(again.body["status"], "processing");
    let queued: i64 = sqlx::query_scalar("SELECT count(*) FROM uploads WHERE status = 'queued'")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(queued, 1);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn posts_only_keep_own_images(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    forum(&db, "general", 0).await;
    let (_, ca) = member(&app, &db).await;
    let html = "<p>x<img src=\"https://evil.example/a.webp\"></p>";
    let r = call(
        &app,
        "POST",
        "/api/v1/forums/general/topics",
        Some(json!({"title": "img", "html": html, "source": {"type": "doc"}})),
        &ca,
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED);
    let topic = r.body["topic_id"].as_i64().unwrap();
    let r = call(&app, "GET", &format!("/api/v1/topics/{topic}"), None, &[]).await;
    assert_eq!(r.body["posts"][0]["html"], "<p>x</p>");
}

/// 给 `user` 记一次拍卖违约（与 worker 判违约同一个函数：违约记录 + 自动禁拍 `ban_secs`），返回禁拍的处罚 ID。
async fn record_default(db: &PgPool, forum_id: i32, seller: i64, user: i64, ban_secs: u32) -> i64 {
    use forum_db::auctions as auctions_db;
    let id = auctions_db::insert_draft(
        db,
        auctions_db::NewAuction {
            seller_id: seller,
            forum_id,
            chain_id: 31_337,
            verifying_contract: &[0x44; 20],
            seller: &[0x11; 20],
            currency: 0,
            cny_per_usdt: 0,
            start_price: "10000000",
            increment: "1000000",
            reserve_price: "10000000",
            duration_secs: 86_400,
            eligibility: 0,
            eligibility_value: None,
            terms: &json!({}),
            content_hash: &[7; 32],
            draft: &json!({}),
        },
    )
    .await
    .unwrap();
    let auction = auctions_db::by_id(db, id).await.unwrap().unwrap();
    let proof = auctions_db::Proof {
        auction_id: id,
        round: 1,
        bid_id: 0,
        bidder_id: user,
        auth_hash: vec![0; 32],
        winner: vec![0x22; 20],
        price: "10000000".into(),
        pay_deadline: time::OffsetDateTime::now_utc(),
        signature: vec![1],
    };
    let mut tx = db.begin().await.unwrap();
    auctions_db::record_default(&mut tx, &auction, &proof, ban_secs)
        .await
        .unwrap()
        .unwrap();
    tx.commit().await.unwrap();
    sqlx::query_scalar("SELECT sanction_id FROM auction_defaults WHERE auction_id = $1")
        .bind(id)
        .fetch_one(db)
        .await
        .unwrap()
}

/// 拍卖违约记录公开：名片与个人主页显示近 12 个月的违约次数与生效中的违约禁拍；解除禁拍后名片缓存失效。
#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn auction_defaults_on_card_and_profile(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let f = forum(&db, "market", 0).await;
    let (seller, _) = member(&app, &db).await;
    let (u, _) = member(&app, &db).await;
    let (clean, _) = member(&app, &db).await;
    let (admin, ac) = member(&app, &db).await;
    staff::grant(&db, admin, staff::ROLE_ADMIN, None, None)
        .await
        .unwrap()
        .unwrap();
    let card = |id: i64| {
        let app = app.clone();
        async move {
            let r = call(
                &app,
                "GET",
                &format!("/api/v1/users/cards?ids={id}"),
                None,
                &[],
            )
            .await;
            assert_eq!(r.status, StatusCode::OK);
            r.body[0]["reputation"].clone()
        }
    };
    let rep = card(clean).await;
    assert_eq!(rep["auction_defaults"], 0);
    assert_eq!(rep["auction_banned_until"], Value::Null);

    // 一次 13 个月前（禁拍早已结束）、一次刚刚（禁拍 30 天）
    let old = record_default(&db, f, seller, u, 86_400).await;
    sqlx::query(
        "UPDATE auction_defaults SET created_at = now() - interval '13 months' WHERE sanction_id = $1",
    )
    .bind(old)
    .execute(&db)
    .await
    .unwrap();
    sqlx::query("UPDATE sanctions SET created_at = now() - interval '13 months', ends_at = now() - interval '12 months' WHERE id = $1")
        .bind(old)
        .execute(&db)
        .await
        .unwrap();
    let ban = record_default(&db, f, seller, u, 30 * 86_400).await;
    let rep = card(u).await;
    assert_eq!(rep["auction_defaults"], 1);
    let until = time::OffsetDateTime::parse(
        rep["auction_banned_until"].as_str().unwrap(),
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap();
    assert!((until - time::OffsetDateTime::now_utc()).whole_days() >= 29);
    // 第二次读来自名片缓存，结果相同
    assert_eq!(card(u).await, rep);
    let r = call(&app, "GET", &format!("/api/v1/users/{u}"), None, &[]).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.body["trade"]["auction_defaults"], 1);
    assert_eq!(
        r.body["trade"]["auction_banned_until"],
        rep["auction_banned_until"]
    );

    // 管理人员手动的禁拍不公开
    forum_db::sanctions::create(
        &db,
        forum_db::sanctions::NewSanction {
            kind: forum_db::sanctions::KIND_AUCTION_BAN,
            user_id: clean,
            forum_id: None,
            ends_at: None,
            reason: "x",
            created_by: Some(admin),
        },
    )
    .await
    .unwrap();
    let r = call(&app, "GET", &format!("/api/v1/users/{clean}"), None, &[]).await;
    assert_eq!(r.body["trade"]["auction_banned_until"], Value::Null);

    // 解除违约禁拍：名片缓存失效，次数仍在
    let r = call(
        &app,
        "DELETE",
        &format!("/api/v1/moderation/sanctions/{ban}?reason=申诉成功"),
        None,
        &ac,
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{:?}", r.body);
    let rep = card(u).await;
    assert_eq!(rep["auction_defaults"], 1);
    assert_eq!(rep["auction_banned_until"], Value::Null);
}
