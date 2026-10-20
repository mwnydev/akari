//! 举报处理队列（范围、举报人身份、处理方式与结案通知）、楼层隐藏（谁看得到什么、隐藏后不能互动）、热门排序、
//! Turnstile 人机验证（登录与新用户发帖，接本机假的 siteverify）。
//! 需要 PostgreSQL（`DATABASE_URL`）与 Valkey（`VALKEY_URL`，未设置则跳过）。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

mod common;

use axum::Router;
use axum::http::StatusCode;
use common::{
    Res, age, build, call, jar, latest_code, namespace, router_of, state_with, unique_email, user,
    wallet_state,
};
use forum_auth::{AnyCaptcha, Turnstile};
use forum_db::forums::{self, NewForum};
use forum_db::{PgPool, staff};
use serde_json::{Value, json};

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
            allow_trade: false,
            allow_auction: false,
            color: None,
        },
    )
    .await
    .unwrap()
}

fn doc() -> Value {
    json!({"type": "doc"})
}

/// 已解除新用户限制的登录用户。
async fn member(app: &Router, db: &PgPool) -> (i64, Vec<String>) {
    let (id, cookies) = user(app, db).await;
    age(db, id).await;
    (id, cookies)
}

async fn grant(db: &PgPool, user_id: i64, role: i16, forum_id: Option<i32>) {
    staff::grant(db, user_id, role, forum_id, None)
        .await
        .unwrap()
        .unwrap();
}

/// 发主题，返回 `(主题 ID, 1 楼 ID)`。
async fn topic(app: &Router, cookies: &[String], slug: &str) -> (i64, i64) {
    let r = call(
        app,
        "POST",
        &format!("/api/v1/forums/{slug}/topics"),
        Some(json!({"title": "主题", "html": "<p>1 楼</p>", "source": doc()})),
        cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    (
        r.body["topic_id"].as_i64().unwrap(),
        r.body["post_id"].as_i64().unwrap(),
    )
}

async fn reply(app: &Router, cookies: &[String], topic: i64, text: &str) -> i64 {
    let r = call(
        app,
        "POST",
        &format!("/api/v1/topics/{topic}/posts"),
        Some(json!({"html": format!("<p>{text}</p>"), "source": doc()})),
        cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    r.body["post_id"].as_i64().unwrap()
}

async fn comment(app: &Router, cookies: &[String], post: i64, body: &str) -> Res {
    call(
        app,
        "POST",
        &format!("/api/v1/posts/{post}/comments"),
        Some(json!({"body": body})),
        cookies,
    )
    .await
}

async fn report(app: &Router, cookies: &[String], path: &str, reason: &str) {
    let r = call(app, "POST", path, Some(json!({"reason": reason})), cookies).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{:?}", r.body);
}

async fn queue(app: &Router, cookies: &[String], query: &str) -> Res {
    call(
        app,
        "GET",
        &format!("/api/v1/moderation/reports{query}"),
        None,
        cookies,
    )
    .await
}

async fn resolve(app: &Router, cookies: &[String], case: i64, body: Value) -> Res {
    call(
        app,
        "POST",
        &format!("/api/v1/moderation/reports/{case}/resolve"),
        Some(body),
        cookies,
    )
    .await
}

fn code(r: &Res) -> (StatusCode, Option<&str>) {
    (r.status, r.body["code"].as_str())
}

/// 队列里某个内容的案件。
fn case_of<'a>(q: &'a Value, kind: &str, id: i64) -> Option<&'a Value> {
    q["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["target"]["kind"] == kind && c["target"]["id"] == id)
}

fn user_ids(q: &Value) -> Vec<i64> {
    q["users"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["id"].as_i64().unwrap())
        .collect()
}

async fn audit_actions(db: &PgPool, actor: i64) -> Vec<String> {
    sqlx::query_scalar("SELECT action FROM admin_audit_logs WHERE actor_id = $1 ORDER BY id")
        .bind(actor)
        .fetch_all(db)
        .await
        .unwrap()
}

/// 发给举报人的结案通知（发件箱 payload）。
async fn resolved_notices(db: &PgPool, to: i64) -> Vec<Value> {
    sqlx::query_scalar(
        "SELECT payload FROM outbox WHERE kind = 'notify.report_resolved' AND payload->>'to' = $1 ORDER BY id",
    )
    .bind(to.to_string())
    .fetch_all(db)
    .await
    .unwrap()
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn report_queue_scope_and_reporter_privacy(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let a = forum(&db, "board-a").await;
    let b = forum(&db, "board-b").await;
    let (author, ac) = member(&app, &db).await;
    let (r1, r1c) = member(&app, &db).await;
    let (r2, r2c) = member(&app, &db).await;
    let (_, plain) = member(&app, &db).await;
    let (mod_a, mc) = member(&app, &db).await;
    let (admin, adc) = member(&app, &db).await;
    grant(&db, mod_a, staff::ROLE_MODERATOR, Some(a)).await;
    grant(&db, admin, staff::ROLE_ADMIN, None).await;

    let (ta, _) = topic(&app, &ac, "board-a").await;
    let p = reply(&app, &ac, ta, "广告").await;
    let c = comment(&app, &ac, p, "加我私聊").await.body["comment"]["id"]
        .as_i64()
        .unwrap();
    let (tb, _) = topic(&app, &ac, "board-b").await;
    let q = reply(&app, &ac, tb, "另一个板块").await;
    report(&app, &r1c, &format!("/api/v1/posts/{p}/report"), "广告").await;
    report(&app, &r2c, &format!("/api/v1/posts/{p}/report"), "刷屏").await;
    // 同一人重复举报不重复计
    report(&app, &r1c, &format!("/api/v1/posts/{p}/report"), "还是广告").await;
    report(&app, &r2c, &format!("/api/v1/comments/{c}/report"), "").await;
    report(&app, &r1c, &format!("/api/v1/posts/{q}/report"), "广告").await;

    // 版主只看自己板块，看不到举报人是谁
    let r = queue(&app, &mc, "").await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert_eq!(r.body["open_count"], 2);
    let pc = case_of(&r.body, "post", p).unwrap();
    assert_eq!(pc["reports"], 2);
    assert_eq!(pc["status"], "open");
    assert_eq!(pc["target"]["floor"], 2);
    assert_eq!(pc["target"]["excerpt"], "广告");
    assert_eq!(pc["target"]["author_id"], author);
    let reasons: Vec<&str> = pc["latest"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["reason"].as_str().unwrap())
        .collect();
    assert_eq!(reasons, ["刷屏", "广告"]);
    assert!(
        pc["latest"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["reporter_id"].is_null())
    );
    assert!(case_of(&r.body, "comment", c).is_some());
    assert!(case_of(&r.body, "post", q).is_none());
    let ids = user_ids(&r.body);
    assert!(ids.contains(&author) && !ids.contains(&r1) && !ids.contains(&r2));

    // 管理员看全部板块与举报人
    let r = queue(&app, &adc, "").await;
    assert_eq!(r.body["open_count"], 3);
    let pc = case_of(&r.body, "post", p).unwrap();
    let reporters: Vec<i64> = pc["latest"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["reporter_id"].as_i64().unwrap())
        .collect();
    assert_eq!(reporters, [r2, r1]);
    assert!(case_of(&r.body, "post", q).is_some());
    assert!(user_ids(&r.body).contains(&r1));

    // 普通用户与游客
    assert_eq!(
        code(&queue(&app, &plain, "").await),
        (StatusCode::FORBIDDEN, Some("FORBIDDEN"))
    );
    assert_eq!(queue(&app, &[], "").await.status, StatusCode::UNAUTHORIZED);

    // 被举报的作者也当上版主：看不到、也处理不了自己内容上的案件
    grant(&db, author, staff::ROLE_MODERATOR, Some(a)).await;
    let r = queue(&app, &ac, "").await;
    assert_eq!(r.body["open_count"], 0);
    assert!(r.body["cases"].as_array().unwrap().is_empty());
    let case = pc["id"].as_i64().unwrap();
    let r = resolve(
        &app,
        &ac,
        case,
        json!({"action": "dismiss", "reason": "不违规"}),
    )
    .await;
    assert_eq!(code(&r), (StatusCode::FORBIDDEN, Some("FORBIDDEN")));

    // 已结案列表起初为空；翻页参数不对的拒绝
    let r = queue(&app, &mc, "?status=resolved").await;
    assert!(r.body["cases"].as_array().unwrap().is_empty());
    assert_eq!(
        queue(&app, &mc, "?status=all").await.status,
        StatusCode::BAD_REQUEST
    );

    // 主题移到 B 板块后，案件跟过去：A 的版主不再看到，管理员照常
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/topics/{ta}/moderation"),
        Some(json!({"action": "move", "reason": "移到 B", "forum_id": b})),
        &adc,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    let r = queue(&app, &mc, "").await;
    assert_eq!(r.body["open_count"], 0);
    let r = resolve(
        &app,
        &mc,
        case,
        json!({"action": "dismiss", "reason": "不违规"}),
    )
    .await;
    assert_eq!(code(&r), (StatusCode::FORBIDDEN, Some("FORBIDDEN")));
    let r = queue(&app, &adc, "").await;
    assert_eq!(
        case_of(&r.body, "post", p).unwrap()["target"]["forum_id"],
        b
    );
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn resolving_reports_acts_audits_and_notifies(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let a = forum(&db, "board-a").await;
    let b = forum(&db, "board-b").await;
    let (author, ac) = member(&app, &db).await;
    let (r1, r1c) = member(&app, &db).await;
    let (r2, r2c) = member(&app, &db).await;
    let (r3, r3c) = member(&app, &db).await;
    let (mod_a, mc) = member(&app, &db).await;
    let (mod_b, mbc) = member(&app, &db).await;
    grant(&db, mod_a, staff::ROLE_MODERATOR, Some(a)).await;
    grant(&db, mod_b, staff::ROLE_MODERATOR, Some(b)).await;

    let (t, _) = topic(&app, &ac, "board-a").await;
    let p = reply(&app, &ac, t, "广告").await;
    let c = comment(&app, &ac, p, "加我私聊").await.body["comment"]["id"]
        .as_i64()
        .unwrap();
    report(&app, &r1c, &format!("/api/v1/posts/{p}/report"), "广告").await;
    report(&app, &r2c, &format!("/api/v1/posts/{p}/report"), "广告").await;
    report(&app, &r1c, &format!("/api/v1/comments/{c}/report"), "").await;
    let cases = queue(&app, &mc, "").await.body;
    let post_case = case_of(&cases, "post", p).unwrap()["id"].as_i64().unwrap();
    let comment_case = case_of(&cases, "comment", c).unwrap()["id"]
        .as_i64()
        .unwrap();

    // 别的板块的版主不能处理；理由必填；处罚要带条款；评论不能隐藏
    let r = resolve(
        &app,
        &mbc,
        post_case,
        json!({"action": "dismiss", "reason": "x"}),
    )
    .await;
    assert_eq!(code(&r), (StatusCode::FORBIDDEN, Some("FORBIDDEN")));
    let r = resolve(
        &app,
        &mc,
        post_case,
        json!({"action": "dismiss", "reason": " "}),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let r = resolve(
        &app,
        &mc,
        post_case,
        json!({"action": "sanction", "reason": "x"}),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let r = resolve(
        &app,
        &mc,
        comment_case,
        json!({"action": "hide", "reason": "x"}),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    // 版主不能封号（处罚本身的权限照常检查）
    let ban = json!({"action": "sanction", "reason": "广告", "sanction": {"kind": "ban"}});
    let r = resolve(&app, &mc, post_case, ban).await;
    assert_eq!(code(&r), (StatusCode::FORBIDDEN, Some("FORBIDDEN")));
    assert!(resolved_notices(&db, r1).await.is_empty());

    // 驳回：两位举报人各收到一条通知，只有处理结果
    let r = resolve(
        &app,
        &mc,
        post_case,
        json!({"action": "dismiss", "reason": "不违规"}),
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{:?}", r.body);
    for reporter in [r1, r2] {
        let n = resolved_notices(&db, reporter).await;
        assert_eq!(n.len(), 1);
        assert_eq!(
            n[0],
            json!({"to": reporter, "outcome": "dismissed", "topic_id": t, "floor": 2})
        );
    }
    let r = resolve(
        &app,
        &mc,
        post_case,
        json!({"action": "hide", "reason": "x"}),
    )
    .await;
    assert_eq!(code(&r), (StatusCode::CONFLICT, Some("REPORT_RESOLVED")));
    let r = queue(&app, &mc, "?status=resolved").await;
    let done = case_of(&r.body, "post", p).unwrap();
    assert_eq!(
        (done["status"].as_str(), done["resolved_by"].as_i64()),
        (Some("dismissed"), Some(mod_a))
    );
    assert!(user_ids(&r.body).contains(&mod_a));

    // 结案后有新的人举报同一楼层：开新的一件；这次隐藏
    report(
        &app,
        &r3c,
        &format!("/api/v1/posts/{p}/report"),
        "确实是广告",
    )
    .await;
    let again = queue(&app, &mc, "").await.body;
    let new_case = case_of(&again, "post", p).unwrap();
    assert_eq!(new_case["reports"], 1);
    let new_case = new_case["id"].as_i64().unwrap();
    assert_ne!(new_case, post_case);
    let r = resolve(
        &app,
        &mc,
        new_case,
        json!({"action": "hide", "reason": "广告"}),
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{:?}", r.body);
    let status: i16 = sqlx::query_scalar("SELECT status FROM posts WHERE id = $1")
        .bind(p)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(status, forum_db::posts::STATUS_HIDDEN);
    assert_eq!(
        resolved_notices(&db, r3).await,
        [json!({"to": r3, "outcome": "actioned", "topic_id": t, "floor": 2})]
    );

    // 删除评论
    let r = resolve(
        &app,
        &mc,
        comment_case,
        json!({"action": "delete", "reason": "引流"}),
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{:?}", r.body);
    let status: i16 = sqlx::query_scalar("SELECT status FROM floor_comments WHERE id = $1")
        .bind(c)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(status, forum_db::floor::STATUS_DELETED_BY_STAFF);
    assert_eq!(
        audit_actions(&db, mod_a).await,
        [
            "report.resolve",
            "post.hide",
            "report.resolve",
            "comment.delete",
            "report.resolve"
        ]
    );
    let detail: Value = sqlx::query_scalar(
        "SELECT detail FROM admin_audit_logs WHERE action = 'report.resolve' AND target = $1",
    )
    .bind(format!("report:{new_case}"))
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(detail["action"], "hide");
    assert_eq!(detail["status"], 2);

    // 处罚作者（版主在本板块禁言 1 天）：作者收到处罚通知，举报人只知道「已处理」
    let (t2, _) = topic(&app, &ac, "board-a").await;
    let p2 = reply(&app, &ac, t2, "又一条广告").await;
    report(&app, &r2c, &format!("/api/v1/posts/{p2}/report"), "广告").await;
    let case = case_of(&queue(&app, &mc, "").await.body, "post", p2).unwrap()["id"]
        .as_i64()
        .unwrap();
    let mute = json!({
        "action": "sanction", "reason": "屡次发广告",
        "sanction": {"kind": "mute", "forum_id": a, "duration_secs": 86_400},
    });
    let r = resolve(&app, &mc, case, mute).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{:?}", r.body);
    let muted: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sanctions WHERE user_id = $1 AND kind = 1 AND forum_id = $2",
    )
    .bind(author)
    .bind(a)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(muted, 1);
    let notice = resolved_notices(&db, r2).await.pop().unwrap();
    assert_eq!(
        notice,
        json!({"to": r2, "outcome": "actioned", "topic_id": t2, "floor": 2})
    );
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/topics/{t2}/posts"),
        Some(json!({"html": "<p>x</p>", "source": doc()})),
        &ac,
    )
    .await;
    assert_eq!(code(&r).1, Some("MUTED"));
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn hidden_posts_visibility_and_interactions(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let a = forum(&db, "board-a").await;
    let b = forum(&db, "board-b").await;
    let (_, ac) = member(&app, &db).await;
    let (_, vc) = member(&app, &db).await;
    let (moderator, mc) = member(&app, &db).await;
    let (other_mod, oc) = member(&app, &db).await;
    grant(&db, moderator, staff::ROLE_MODERATOR, Some(a)).await;
    grant(&db, other_mod, staff::ROLE_MODERATOR, Some(b)).await;
    let (t, first) = topic(&app, &ac, "board-a").await;
    let p = reply(&app, &ac, t, "秘密内容").await;
    assert_eq!(
        comment(&app, &vc, p, "沙发").await.status,
        StatusCode::CREATED
    );

    let hide = |cookies: Vec<String>, post: i64, reason: &str| {
        let (app, body) = (app.clone(), json!({ "reason": reason }));
        async move {
            call(
                &app,
                "POST",
                &format!("/api/v1/posts/{post}/hide"),
                Some(body),
                &cookies,
            )
            .await
        }
    };
    assert_eq!(
        code(&hide(vc.clone(), p, "x").await),
        (StatusCode::FORBIDDEN, Some("FORBIDDEN"))
    );
    assert_eq!(
        code(&hide(oc.clone(), p, "x").await),
        (StatusCode::FORBIDDEN, Some("FORBIDDEN"))
    );
    assert_eq!(
        hide(mc.clone(), p, "").await.status,
        StatusCode::BAD_REQUEST
    );
    // 1 楼请隐藏主题
    assert_eq!(
        hide(mc.clone(), first, "x").await.status,
        StatusCode::BAD_REQUEST
    );
    let r = hide(mc.clone(), p, "含联系方式").await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{:?}", r.body);
    assert_eq!(
        hide(mc.clone(), p, "again").await.status,
        StatusCode::NOT_FOUND
    );

    let page = |cookies: Vec<String>| {
        let app = app.clone();
        async move {
            let r = call(&app, "GET", &format!("/api/v1/topics/{t}"), None, &cookies).await;
            assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
            (r.body["posts"][1].clone(), r.body["social"][1].clone())
        }
    };
    // 游客与其他用户：只知道被隐藏，看不到内容、理由与评论
    for cookies in [Vec::new(), vc.clone()] {
        let (post, social) = page(cookies).await;
        assert_eq!(post["hidden"], true);
        assert_eq!(post["deleted"], false);
        assert_eq!(post["html"], "");
        assert!(post["hidden_reason"].is_null());
        assert_eq!(social["comment_count"], 0);
        assert!(social["comments"].as_array().unwrap().is_empty());
    }
    // 作者与管理人员：看得到内容与理由
    for cookies in [ac.clone(), mc.clone()] {
        let (post, social) = page(cookies).await;
        assert_eq!(post["hidden"], true);
        assert_eq!(post["html"], "<p>秘密内容</p>");
        assert_eq!(post["hidden_reason"], "含联系方式");
        assert_eq!(social["comment_count"], 1);
    }
    // 编辑历史与评论翻页同样只给作者与管理人员
    for (cookies, ok) in [(vc.clone(), false), (ac.clone(), true), (mc.clone(), true)] {
        for path in [
            format!("/api/v1/posts/{p}/history"),
            format!("/api/v1/posts/{p}/comments"),
        ] {
            let r = call(&app, "GET", &path, None, &cookies).await;
            assert_eq!(r.status == StatusCode::OK, ok, "{path}: {:?}", r.body);
        }
    }

    // 隐藏楼层不能点赞、评论、举报，作者也不能编辑或自己删除
    let like = call(&app, "PUT", &format!("/api/v1/posts/{p}/like"), None, &vc).await;
    assert_eq!(like.status, StatusCode::NOT_FOUND);
    assert_eq!(
        comment(&app, &vc, p, "再来").await.status,
        StatusCode::NOT_FOUND
    );
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/posts/{p}/report"),
        Some(json!({})),
        &vc,
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    let edit = json!({"html": "<p>改</p>", "source": doc()});
    let r = call(
        &app,
        "PATCH",
        &format!("/api/v1/posts/{p}"),
        Some(edit),
        &ac,
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    let r = call(&app, "DELETE", &format!("/api/v1/posts/{p}"), None, &ac).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);

    // 管理人员可以删除隐藏楼层；恢复后仍是隐藏的
    let r = call(
        &app,
        "DELETE",
        &format!("/api/v1/posts/{p}?reason=spam"),
        None,
        &mc,
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{:?}", r.body);
    assert_eq!(page(ac.clone()).await.0["deleted"], true);
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/posts/{p}/restore"),
        Some(json!({"reason": "误删"})),
        &mc,
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{:?}", r.body);
    assert_eq!(page(vc.clone()).await.0["hidden"], true);

    // 取消隐藏：所有人恢复可见，审计日志记下原隐藏理由
    let unhide = json!({"reason": "已整改"});
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/posts/{p}/unhide"),
        Some(unhide.clone()),
        &oc,
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/posts/{p}/unhide"),
        Some(unhide.clone()),
        &mc,
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{:?}", r.body);
    let (post, _) = page(vc.clone()).await;
    assert_eq!(
        (post["hidden"].as_bool(), post["html"].as_str()),
        (Some(false), Some("<p>秘密内容</p>"))
    );
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/posts/{p}/unhide"),
        Some(unhide),
        &mc,
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert_eq!(
        audit_actions(&db, moderator).await,
        ["post.hide", "post.delete", "post.restore", "post.unhide"]
    );
    let detail: Value = sqlx::query_scalar(
        "SELECT detail FROM admin_audit_logs WHERE action = 'post.unhide' AND actor_id = $1",
    )
    .bind(moderator)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(detail["hidden_reason"], "含联系方式");
}

/// 用 SQL 直接摆好热度：发帖时间、回复数（楼层数）与 1 楼点赞数。
async fn shape(db: &PgPool, topic: i64, first_post: i64, hours_ago: i64, floors: i32, likes: i32) {
    sqlx::query(
        "UPDATE topics SET created_at = now() - make_interval(hours => $2::int),
                           last_post_at = now() - make_interval(hours => $2::int), last_floor = $3
         WHERE id = $1",
    )
    .bind(topic)
    .bind(hours_ago)
    .bind(floors)
    .execute(db)
    .await
    .unwrap();
    sqlx::query("INSERT INTO post_counters (post_id, likes) VALUES ($1, $2)")
        .bind(first_post)
        .bind(likes)
        .execute(db)
        .await
        .unwrap();
}

fn topic_ids(body: &Value) -> Vec<i64> {
    body["topics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_i64().unwrap())
        .collect()
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn hot_sort_orders_by_decayed_activity(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    forum(&db, "board-a").await;
    forum(&db, "board-b").await;
    let hidden_board = forum(&db, "staff-only").await;
    sqlx::query("UPDATE forums SET view_min_level = 9 WHERE id = $1")
        .bind(hidden_board)
        .execute(&db)
        .await
        .unwrap();
    let (_, ac) = member(&app, &db).await;
    let (admin, adc) = member(&app, &db).await;
    grant(&db, admin, staff::ROLE_ADMIN, None).await;

    // 热度 = (楼层数 + 2 × 1 楼赞) / (小时 + 2)^1.5
    let (busy, busy_p) = topic(&app, &ac, "board-a").await; // (40 + 20) / 6^1.5 ≈ 4.08
    shape(&db, busy, busy_p, 4, 40, 10).await;
    let (fresh, fresh_p) = topic(&app, &ac, "board-a").await; // (3 + 0) / 2.83 ≈ 1.06
    shape(&db, fresh, fresh_p, 0, 3, 0).await;
    let (liked, liked_p) = topic(&app, &ac, "board-b").await; // (1 + 60) / 26^1.5 ≈ 0.46
    shape(&db, liked, liked_p, 24, 1, 30).await;
    let (old, old_p) = topic(&app, &ac, "board-a").await; // 15 天前：不进排名
    shape(&db, old, old_p, 15 * 24, 500, 100).await;
    let (secret, secret_p) = topic(&app, &adc, "staff-only").await;
    shape(&db, secret, secret_p, 1, 100, 0).await;
    let (gone, gone_p) = topic(&app, &ac, "board-a").await;
    shape(&db, gone, gone_p, 1, 90, 0).await;
    assert_eq!(forum_db::hot::refresh(&db).await.unwrap(), 5);
    // 排名之后才隐藏的主题按当前状态过滤
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/topics/{gone}/moderation"),
        Some(json!({"action": "hide", "reason": "x"})),
        &adc,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);

    let r = call(
        &app,
        "GET",
        "/api/v1/forums/board-a/topics?sort=hot",
        None,
        &[],
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert_eq!(topic_ids(&r.body), [busy, fresh]);
    assert!(r.body["pinned"].as_array().unwrap().is_empty());
    assert!(r.body["next_cursor"].is_null() && r.body["next_page"].is_null());
    let r = call(&app, "GET", "/api/v1/topics/latest?sort=hot", None, &[]).await;
    assert_eq!(topic_ids(&r.body), [busy, fresh, liked]);
    // 看得到隐藏板块的人才有那个板块的主题
    let r = call(&app, "GET", "/api/v1/topics/latest?sort=hot", None, &adc).await;
    assert_eq!(topic_ids(&r.body), [secret, busy, fresh, liked]);
    // 默认仍是最新列表（含热门排不进的老主题）
    let r = call(&app, "GET", "/api/v1/forums/board-a/topics", None, &[]).await;
    assert_eq!(topic_ids(&r.body).len(), 3);
    assert!(topic_ids(&r.body).contains(&old));
    let r = call(
        &app,
        "GET",
        "/api/v1/forums/board-a/topics?sort=hot&page=2",
        None,
        &[],
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let r = call(
        &app,
        "GET",
        "/api/v1/forums/board-a/topics?sort=hot&cursor=nope",
        None,
        &[],
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);

    // 翻页：50 个一页，按 (分数, ID) 游标（直接建主题，避开发帖限频）
    let board_b = forums::by_slug(&db, "board-b").await.unwrap().unwrap().id;
    for i in 0..55 {
        let created = forum_db::topics::create(
            &db,
            forum_db::topics::NewTopic {
                forum_id: board_b,
                author_id: admin,
                title: "批量",
                html: "<p>x</p>",
                plain_text: "x",
                source: &doc(),
                mentions: &[],
                kind: forum_db::topics::KIND_NORMAL,
            },
        )
        .await
        .unwrap();
        shape(&db, created.topic_id, created.post_id, 30 + i, 2, 0).await;
    }
    forum_db::hot::refresh(&db).await.unwrap();
    let r = call(
        &app,
        "GET",
        "/api/v1/forums/board-b/topics?sort=hot",
        None,
        &[],
    )
    .await;
    let first = topic_ids(&r.body);
    assert_eq!(first.len(), 50);
    assert_eq!(first[0], liked);
    let cursor = r.body["next_cursor"].as_str().unwrap().to_owned();
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/forums/board-b/topics?sort=hot&cursor={cursor}"),
        None,
        &[],
    )
    .await;
    let second = topic_ids(&r.body);
    assert_eq!(second.len(), 6);
    assert!(second.iter().all(|t| !first.contains(t)));
    assert!(r.body["next_cursor"].is_null());
}

/// 本机假的 Turnstile siteverify：令牌 `ok:<action>` 通过并带回该 action，其他令牌不通过。
async fn fake_siteverify() -> String {
    use axum::routing::post;
    async fn verify(axum::Json(req): axum::Json<Value>) -> axum::Json<Value> {
        assert_eq!(req["secret"], "test-secret");
        assert!(req["remoteip"].as_str().unwrap().starts_with("10."));
        let token = req["response"].as_str().unwrap_or_default();
        match token.strip_prefix("ok:") {
            Some(action) => axum::Json(json!({"success": true, "action": action})),
            None => {
                axum::Json(json!({"success": false, "error-codes": ["invalid-input-response"]}))
            }
        }
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = axum::Router::new().route("/siteverify", post(verify));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}/siteverify")
}

fn turnstile(endpoint: String) -> AnyCaptcha {
    // 连本机：不走 shell 里的 HTTP 代理
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(forum_auth::captcha::TIMEOUT)
        .build()
        .unwrap();
    AnyCaptcha::Turnstile(Turnstile::with_endpoint(
        client,
        "test-secret".to_owned(),
        endpoint,
    ))
}

async fn start_login(app: &Router, email: &str, captcha: Option<&str>) -> Res {
    call(
        app,
        "POST",
        "/api/v1/auth/email/start",
        Some(json!({"email": email, "captcha": captcha})),
        &[],
    )
    .await
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn turnstile_guards_login_and_new_user_posting(db: PgPool) {
    let Some(mut state) = state_with(db.clone(), wallet_state(), &namespace()) else {
        return;
    };
    state.captcha = turnstile(fake_siteverify().await);
    let app = router_of(state);
    forum(&db, "board-a").await;

    // 登录：没有令牌、令牌无效、或令牌来自发帖表单都拒绝
    let email = unique_email();
    for token in [None, Some("bad"), Some("ok:post")] {
        let r = start_login(&app, &email, token).await;
        assert_eq!(
            code(&r),
            (StatusCode::FORBIDDEN, Some("CAPTCHA_FAILED")),
            "{token:?}"
        );
    }
    let r = start_login(&app, &email, Some("ok:login")).await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{:?}", r.body);
    let code_ = latest_code(&db).await;
    let r = call(
        &app,
        "POST",
        "/api/v1/auth/email/verify",
        Some(json!({"email": email, "code": code_})),
        &[],
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    let id = r.body["user"]["id"].as_i64().unwrap();
    assert_eq!(r.body["user"]["new_user"], true);
    let cookies = jar(&r.cookies);

    // 新用户发帖：没带令牌提示先验证，令牌无效或来自登录表单都拒绝
    let new_topic = |captcha: Option<&str>| {
        let app = app.clone();
        let cookies = cookies.clone();
        let body =
            json!({"title": "新人帖", "html": "<p>你好</p>", "source": doc(), "captcha": captcha});
        async move {
            call(
                &app,
                "POST",
                "/api/v1/forums/board-a/topics",
                Some(body),
                &cookies,
            )
            .await
        }
    };
    assert_eq!(
        code(&new_topic(None).await),
        (StatusCode::FORBIDDEN, Some("CAPTCHA_REQUIRED"))
    );
    assert_eq!(code(&new_topic(Some("")).await).1, Some("CAPTCHA_REQUIRED"));
    assert_eq!(
        code(&new_topic(Some("bad")).await).1,
        Some("CAPTCHA_FAILED")
    );
    assert_eq!(
        code(&new_topic(Some("ok:login")).await).1,
        Some("CAPTCHA_FAILED")
    );
    let r = new_topic(Some("ok:post")).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    let t = r.body["topic_id"].as_i64().unwrap();

    let reply = |captcha: Option<&str>| {
        let app = app.clone();
        let cookies = cookies.clone();
        let body = json!({"html": "<p>回复</p>", "source": doc(), "captcha": captcha});
        async move {
            call(
                &app,
                "POST",
                &format!("/api/v1/topics/{t}/posts"),
                Some(body),
                &cookies,
            )
            .await
        }
    };
    assert_eq!(code(&reply(None).await).1, Some("CAPTCHA_REQUIRED"));
    let r = reply(Some("ok:post")).await;
    assert_eq!(r.status, StatusCode::CREATED);
    // 评论回帖（主楼不能评论）
    let post = r.body["post_id"].as_i64().unwrap();
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/posts/{post}/comments"),
        Some(json!({"body": "评论"})),
        &cookies,
    )
    .await;
    assert_eq!(code(&r).1, Some("CAPTCHA_REQUIRED"));
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/posts/{post}/comments"),
        Some(json!({"body": "评论", "captcha": "ok:post"})),
        &cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);

    // 注册满 24 小时后不再需要
    age(&db, id).await;
    assert_eq!(
        call(&app, "GET", "/api/v1/me", None, &cookies).await.body["new_user"],
        false
    );
    assert_eq!(new_topic(None).await.status, StatusCode::CREATED);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn turnstile_outage_rejects(db: PgPool) {
    let Some(mut state) = state_with(db.clone(), wallet_state(), &namespace()) else {
        return;
    };
    // 没有任何服务在听的本机端口
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/siteverify", closed.local_addr().unwrap());
    drop(closed);
    state.captcha = turnstile(endpoint);
    let app = router_of(state);
    let r = start_login(&app, &unique_email(), Some("ok:login")).await;
    assert_eq!(
        code(&r),
        (StatusCode::INTERNAL_SERVER_ERROR, Some("INTERNAL"))
    );
    let sent: i64 =
        sqlx::query_scalar("SELECT count(*) FROM outbox WHERE kind = 'email.login_code'")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(sent, 0);
}
