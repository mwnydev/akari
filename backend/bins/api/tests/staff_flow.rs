//! 管理角色、权限边界与兜底措施（ADR 0013）：版主只管自己板块、处罚时长与角色高低、封禁的效果、
//! 管理人员新设备告警、限速与异常暂停、站长后台（Passkey、独立 Cookie、再验证、任免、阈值、一键撤销、仲裁总览）。
//! 需要 PostgreSQL（`DATABASE_URL`）与 Valkey（`VALKEY_URL`，未设置则跳过）。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

mod common;

use std::net::SocketAddr;

use axum::Router;
use axum::body::Body;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{Request, StatusCode, header};
use common::{
    ADMIN_ORIGIN, Res, SECRET, SoftAuth, WALLET_ORIGIN, age, build, call, call_from, clear_resend,
    jar, latest_code, namespace, rand_ip, router_of, set_username, soft_passkey, state_with,
    unique_email, user, wallet_state,
};
use forum_auth::passkey::webauthn::{CreationChallengeResponse, RequestChallengeResponse, Url};
use forum_core::staff::ModerationSettings;
use forum_db::forums::{self, NewForum};
use forum_db::{PgPool, staff};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use time::format_description::well_known::Rfc3339;
use tower::ServiceExt;

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
            color: None,
        },
    )
    .await
    .unwrap()
}

fn doc(text: &str) -> Value {
    json!({"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": text}]}]})
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

async fn post_topic(app: &Router, cookies: &[String], slug: &str) -> Res {
    call(
        app,
        "POST",
        &format!("/api/v1/forums/{slug}/topics"),
        Some(json!({"title": "t", "html": "<p>x</p>", "source": doc("x")})),
        cookies,
    )
    .await
}

async fn topic(app: &Router, cookies: &[String], slug: &str) -> i64 {
    let r = post_topic(app, cookies, slug).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    r.body["topic_id"].as_i64().unwrap()
}

async fn reply(app: &Router, cookies: &[String], topic: i64) -> Res {
    call(
        app,
        "POST",
        &format!("/api/v1/topics/{topic}/posts"),
        Some(json!({"html": "<p>r</p>", "source": doc("r")})),
        cookies,
    )
    .await
}

async fn reply_id(app: &Router, cookies: &[String], topic: i64) -> i64 {
    let r = reply(app, cookies, topic).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    r.body["post_id"].as_i64().unwrap()
}

async fn moderate(app: &Router, cookies: &[String], topic: i64, action: &str) -> Res {
    call(
        app,
        "POST",
        &format!("/api/v1/topics/{topic}/moderation"),
        Some(json!({"action": action, "reason": "测试"})),
        cookies,
    )
    .await
}

async fn delete_post(app: &Router, cookies: &[String], post: i64) -> Res {
    call(
        app,
        "DELETE",
        &format!("/api/v1/posts/{post}?reason=spam"),
        None,
        cookies,
    )
    .await
}

async fn sanction(app: &Router, cookies: &[String], user: i64, body: Value) -> Res {
    call(
        app,
        "POST",
        &format!("/api/v1/moderation/users/{user}/sanctions"),
        Some(body),
        cookies,
    )
    .await
}

fn code(r: &Res) -> (StatusCode, Option<&str>) {
    (r.status, r.body["code"].as_str())
}

const DAY: i64 = 86_400;

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn me_reports_roles_and_permissions(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let f = forum(&db, "general").await;
    let (_, plain) = member(&app, &db).await;
    let (m_id, m) = member(&app, &db).await;
    let (a_id, a) = member(&app, &db).await;
    let (o_id, o) = member(&app, &db).await;
    grant(&db, m_id, staff::ROLE_MODERATOR, Some(f)).await;
    grant(&db, a_id, staff::ROLE_ADMIN, None).await;
    grant(&db, o_id, staff::ROLE_OWNER, None).await;
    let me = |c: Vec<String>| {
        let app = app.clone();
        async move { call(&app, "GET", "/api/v1/me", None, &c).await.body }
    };

    assert!(me(plain).await["staff"].is_null());
    let s = me(m).await["staff"].clone();
    assert_eq!(s["roles"], json!(["moderator"]));
    assert_eq!(s["boards"], json!([f]));
    assert_eq!(s["site_permissions"], json!([]));
    let board: Vec<&str> = s["board_permissions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap())
        .collect();
    for p in [
        "topic_moderate",
        "topic_hide",
        "post_delete",
        "mute",
        "auction_ban",
    ] {
        assert!(board.contains(&p), "{p}");
    }
    assert!(!board.contains(&"ban") && !board.contains(&"post_edit"));
    assert_eq!(s["max_mute_secs"], 7 * DAY);
    assert_eq!(s["max_auction_ban_secs"], 30 * DAY);
    assert_eq!(s["admin_backend"], false);

    let s = me(a).await["staff"].clone();
    assert_eq!(s["roles"], json!(["admin"]));
    let site = s["site_permissions"].as_array().unwrap();
    assert!(site.contains(&json!("ban")) && site.contains(&json!("ip_ban")));
    assert!(!site.contains(&json!("manage_staff")));
    assert!(s["max_mute_secs"].is_null());
    assert_eq!(s["admin_backend"], false);

    let s = me(o).await["staff"].clone();
    assert_eq!(s["roles"], json!(["owner"]));
    assert_eq!(s["admin_backend"], true);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn moderator_limited_to_own_board(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let mine = forum(&db, "mine").await;
    let other = forum(&db, "other").await;
    let (_, a) = member(&app, &db).await;
    let (m_id, m) = member(&app, &db).await;
    grant(&db, m_id, staff::ROLE_MODERATOR, Some(mine)).await;
    let t_mine = topic(&app, &a, "mine").await;
    let t_other = topic(&app, &a, "other").await;
    let p_other = reply_id(&app, &a, t_other).await;
    let p_mine = reply_id(&app, &a, t_mine).await;

    for action in ["pin", "feature", "lock", "hide"] {
        let r = moderate(&app, &m, t_mine, action).await;
        assert_eq!(r.status, StatusCode::OK, "{action} {:?}", r.body);
        assert_eq!(
            moderate(&app, &m, t_other, action).await.status,
            StatusCode::FORBIDDEN,
            "{action}"
        );
    }
    // 移动到不归自己管的板块
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/topics/{t_mine}/moderation"),
        Some(json!({"action": "move", "reason": "x", "forum_id": other})),
        &m,
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert_eq!(
        delete_post(&app, &m, p_other).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        delete_post(&app, &m, p_mine).await.status,
        StatusCode::NO_CONTENT
    );
    // 恢复也只在自己板块
    let restore = |c: Vec<String>, p: i64| {
        let app = app.clone();
        async move {
            call(
                &app,
                "POST",
                &format!("/api/v1/posts/{p}/restore"),
                Some(json!({"reason": "误删"})),
                &c,
            )
            .await
        }
    };
    assert_eq!(
        restore(m.clone(), p_mine).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        restore(m.clone(), p_mine).await.status,
        StatusCode::NOT_FOUND
    );
    // 普通用户不能恢复（主题已被隐藏，对普通用户表现为不存在）
    assert_eq!(
        restore(a.clone(), p_mine).await.status,
        StatusCode::NOT_FOUND
    );
    // 版主看不到别的板块的隐藏主题
    let (a2_id, a2) = member(&app, &db).await;
    grant(&db, a2_id, staff::ROLE_ADMIN, None).await;
    assert_eq!(
        moderate(&app, &a2, t_other, "hide").await.status,
        StatusCode::OK
    );
    assert_eq!(
        call(&app, "GET", &format!("/api/v1/topics/{t_other}"), None, &m)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&app, "GET", &format!("/api/v1/topics/{t_mine}"), None, &m)
            .await
            .status,
        StatusCode::OK
    );
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn sanction_durations_scopes_and_ranks(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let mine = forum(&db, "mine").await;
    let other = forum(&db, "other").await;
    let (u, uc) = member(&app, &db).await;
    let (m_id, m) = member(&app, &db).await;
    let (m2_id, _) = member(&app, &db).await;
    let (a_id, a) = member(&app, &db).await;
    let (a2_id, _) = member(&app, &db).await;
    let (o_id, o) = member(&app, &db).await;
    grant(&db, m_id, staff::ROLE_MODERATOR, Some(mine)).await;
    grant(&db, m2_id, staff::ROLE_MODERATOR, Some(other)).await;
    grant(&db, a_id, staff::ROLE_ADMIN, None).await;
    grant(&db, a2_id, staff::ROLE_ADMIN, None).await;
    grant(&db, o_id, staff::ROLE_OWNER, None).await;
    let mute = |forum_id: Option<i32>, secs: Option<i64>| json!({"kind": "mute", "forum_id": forum_id, "duration_secs": secs, "reason": "刷屏"});

    // 版主：自己板块禁言 ≤ 7 天
    let r = sanction(&app, &m, u, mute(Some(mine), Some(7 * DAY))).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    let mute_id = r.body["ids"][0].as_i64().unwrap();
    let r = sanction(&app, &m, u, mute(Some(mine), Some(8 * DAY))).await;
    assert_eq!(
        code(&r),
        (StatusCode::BAD_REQUEST, Some("SANCTION_TOO_LONG"))
    );
    let r = sanction(&app, &m, u, mute(Some(mine), None)).await;
    assert_eq!(
        code(&r),
        (StatusCode::BAD_REQUEST, Some("SANCTION_TOO_LONG"))
    );
    // 全站禁言、别的板块、封号、IP 封禁都不行
    assert_eq!(
        sanction(&app, &m, u, mute(None, Some(DAY))).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        sanction(&app, &m, u, mute(Some(other), Some(DAY)))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    let ban = json!({"kind": "ban", "duration_secs": null, "reason": "诈骗"});
    assert_eq!(
        sanction(&app, &m, u, ban.clone()).await.status,
        StatusCode::FORBIDDEN
    );
    let r = call(
        &app,
        "POST",
        "/api/v1/moderation/ip-bans",
        Some(json!({"ip": "203.0.113.9", "reason": "x"})),
        &m,
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    // 禁拍 ≤ 30 天
    let ab = |secs: i64| json!({"kind": "auction_ban", "forum_id": mine, "duration_secs": secs, "reason": "违约"});
    assert_eq!(
        sanction(&app, &m, u, ab(30 * DAY)).await.status,
        StatusCode::CREATED
    );
    assert_eq!(
        code(&sanction(&app, &m, u, ab(30 * DAY + 1)).await).1,
        Some("SANCTION_TOO_LONG")
    );
    // 太短、封号限定板块：参数错误
    assert_eq!(
        sanction(&app, &m, u, mute(Some(mine), Some(10)))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    let r = sanction(
        &app,
        &a,
        u,
        json!({"kind": "ban", "forum_id": mine, "reason": "x"}),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);

    // 角色高低：版主不能禁言版主、管理员；管理员不能封管理员、站长；不能处罚自己
    assert_eq!(
        sanction(&app, &m, m2_id, mute(Some(mine), Some(DAY)))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        sanction(&app, &m, a_id, mute(Some(mine), Some(DAY)))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        sanction(&app, &a, a2_id, ban.clone()).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        sanction(&app, &a, o_id, ban.clone()).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        sanction(&app, &a, a_id, ban.clone()).await.status,
        StatusCode::FORBIDDEN
    );
    // 管理员：全站、永久；站长可以处罚管理员
    assert_eq!(
        sanction(&app, &a, u, mute(None, None)).await.status,
        StatusCode::CREATED
    );
    let r = sanction(&app, &o, a2_id, mute(Some(mine), Some(DAY))).await;
    assert_eq!(r.status, StatusCode::CREATED);
    let owner_mute = r.body["ids"][0].as_i64().unwrap();

    // 解除：版主解不了站长对管理员的禁言，能解自己板块对普通用户的
    let lift = |c: Vec<String>, id: i64| {
        let app = app.clone();
        async move {
            call(
                &app,
                "DELETE",
                &format!("/api/v1/moderation/sanctions/{id}?reason=ok"),
                None,
                &c,
            )
            .await
        }
    };
    assert_eq!(
        lift(m.clone(), owner_mute).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        lift(m.clone(), mute_id).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(lift(m.clone(), mute_id).await.status, StatusCode::NOT_FOUND);

    // 处罚记录：管理人员可看，普通用户不行；被处罚的人收到通知
    let path = format!("/api/v1/moderation/users/{u}/sanctions");
    assert_eq!(
        call(&app, "GET", &path, None, &uc).await.status,
        StatusCode::FORBIDDEN
    );
    let list = call(&app, "GET", &path, None, &m).await.body;
    let rows = list["sanctions"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows.iter().filter(|s| s["active"] == true).count(), 2);
    let notices: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox WHERE kind = 'notify.sanction' AND payload->>'to' = $1",
    )
    .bind(u.to_string())
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(notices, 3);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn mutes_and_bans_take_effect(db: PgPool) {
    let Some(state) = state_with(db.clone(), wallet_state(), &namespace()) else {
        return;
    };
    // 已知的客户端 IP，测试 IP 封禁（每次运行不同：登录按 IP 限流的键不分测试命名空间）
    let ip = rand_ip();
    let app = forum_api::app(state, &SECRET).layer(MockConnectInfo(SocketAddr::from((ip, 4000))));
    let mine = forum(&db, "mine").await;
    forum(&db, "other").await;
    let (u, uc) = member(&app, &db).await;
    let (peer, _) = member(&app, &db).await;
    let (m_id, m) = member(&app, &db).await;
    let (a_id, a) = member(&app, &db).await;
    grant(&db, m_id, staff::ROLE_MODERATOR, Some(mine)).await;
    grant(&db, a_id, staff::ROLE_ADMIN, None).await;
    let t = topic(&app, &uc, "mine").await;
    let own_post: i64 = sqlx::query_scalar("SELECT id FROM posts WHERE topic_id = $1")
        .bind(t)
        .fetch_one(&db)
        .await
        .unwrap();

    // 板块禁言：本板块不能发帖、回帖、编辑，别的板块照常
    let r = sanction(
        &app,
        &m,
        u,
        json!({"kind": "mute", "forum_id": mine, "duration_secs": DAY, "reason": "x"}),
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED);
    let mute_id = r.body["ids"][0].as_i64().unwrap();
    assert_eq!(code(&post_topic(&app, &uc, "mine").await).1, Some("MUTED"));
    assert_eq!(code(&reply(&app, &uc, t).await).1, Some("MUTED"));
    let edit = call(
        &app,
        "PATCH",
        &format!("/api/v1/posts/{own_post}"),
        Some(json!({"html": "<p>y</p>", "source": doc("y")})),
        &uc,
    )
    .await;
    assert_eq!(code(&edit).1, Some("MUTED"));
    assert_eq!(
        post_topic(&app, &uc, "other").await.status,
        StatusCode::CREATED
    );
    // 到期即失效
    sqlx::query("UPDATE sanctions SET created_at = now() - interval '2 days', ends_at = now() - interval '1 second' WHERE id = $1")
        .bind(mute_id)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(reply(&app, &uc, t).await.status, StatusCode::CREATED);

    // 封号：哪里都不能发帖、不能发私信；仍能登录和读
    let r = sanction(&app, &a, u, json!({"kind": "ban", "reason": "诈骗"})).await;
    assert_eq!(r.status, StatusCode::CREATED);
    let ban_id = r.body["ids"][0].as_i64().unwrap();
    assert_eq!(
        code(&post_topic(&app, &uc, "other").await).1,
        Some("ACCOUNT_BANNED")
    );
    let r = call(
        &app,
        "POST",
        "/api/v1/conversations",
        Some(json!({"user_id": peer})),
        &uc,
    )
    .await;
    assert_eq!(code(&r).1, Some("ACCOUNT_BANNED"));
    assert_eq!(
        call(&app, "GET", "/api/v1/me", None, &uc).await.status,
        StatusCode::OK
    );
    let r = call(
        &app,
        "DELETE",
        &format!("/api/v1/moderation/sanctions/{ban_id}?reason=申诉成功"),
        None,
        &a,
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert_eq!(
        post_topic(&app, &uc, "other").await.status,
        StatusCode::CREATED
    );

    // 设备封禁：该用户登录过的设备不能再发验证码
    let r = sanction(&app, &a, u, json!({"kind": "device_ban", "reason": "换号"})).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    let start = |cookies: Vec<String>| {
        let app = app.clone();
        async move {
            call(
                &app,
                "POST",
                "/api/v1/auth/email/start",
                Some(json!({"email": unique_email()})),
                &cookies,
            )
            .await
        }
    };
    assert_eq!(code(&start(uc.clone()).await).1, Some("ACCESS_BANNED"));
    assert_eq!(start(vec![]).await.status, StatusCode::ACCEPTED);

    // IP 封禁：网段内一律不能登录；版主不能封，管理员可以；网段太大拒绝
    let ip_ban = |c: Vec<String>, ip: String| {
        let app = app.clone();
        async move {
            call(
                &app,
                "POST",
                "/api/v1/moderation/ip-bans",
                Some(json!({"ip": ip, "duration_secs": 3600, "reason": "撞库"})),
                &c,
            )
            .await
        }
    };
    assert_eq!(
        ip_ban(a.clone(), "10.0.0.0/8".into()).await.status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        ip_ban(m.clone(), format!("{}.{}.0.0/16", ip[0], ip[1]))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        ip_ban(a.clone(), format!("{}.{}.9.9/16", ip[0], ip[1]))
            .await
            .status,
        StatusCode::CREATED
    );
    let cidr: String = sqlx::query_scalar("SELECT ip::text FROM sanctions WHERE kind = 4")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(cidr, format!("{}.{}.0.0/16", ip[0], ip[1]));
    assert_eq!(code(&start(vec![]).await).1, Some("ACCESS_BANNED"));
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn new_device_login_alerts_staff_only(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let f = forum(&db, "general").await;
    let staff_email = unique_email().to_lowercase();
    let user_email = unique_email().to_lowercase();
    let r = common::login(&app, &db, &staff_email, &[]).await;
    let m_id = r.body["user"]["id"].as_i64().unwrap();
    common::login(&app, &db, &user_email, &[]).await;
    grant(&db, m_id, staff::ROLE_MODERATOR, Some(f)).await;
    let alerts = || async {
        sqlx::query_scalar::<_, String>(
            "SELECT payload->>'text' FROM outbox WHERE kind = 'staff_alert' AND payload->>'event' = 'staff.new_device'",
        )
        .fetch_all(&db)
        .await
        .unwrap()
    };

    // 普通用户换设备：只发本人邮件
    clear_resend(&user_email).await;
    common::login(&app, &db, &user_email, &[]).await;
    assert!(alerts().await.is_empty());

    // 管理人员换设备：本人邮件 + 管理员群
    clear_resend(&staff_email).await;
    let r = common::login(&app, &db, &staff_email, &[]).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.body["user"]["staff"]["roles"], json!(["moderator"]));
    let texts = alerts().await;
    assert_eq!(texts.len(), 1);
    assert!(
        texts[0].contains(&format!("#{m_id}")) && texts[0].contains("版主"),
        "{}",
        texts[0]
    );
    let mails: i64 =
        sqlx::query_scalar("SELECT count(*) FROM outbox WHERE kind = 'email.new_device'")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(mails, 2);

    // 同一设备再登录不告警
    clear_resend(&staff_email).await;
    common::login(&app, &db, &staff_email, &jar(&r.cookies)).await;
    assert_eq!(alerts().await.len(), 1);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn rate_limit_and_anomaly_suspension(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let f = forum(&db, "general").await;
    let (_, a) = member(&app, &db).await;
    let (m_id, m) = member(&app, &db).await;
    grant(&db, m_id, staff::ROLE_MODERATOR, Some(f)).await;
    forum_db::settings::set_moderation(
        &db,
        &ModerationSettings {
            rate_content: 1,
            anomaly_deletes: 5,
            ..ModerationSettings::default()
        },
        m_id,
    )
    .await
    .unwrap();
    let t = topic(&app, &a, "general").await;

    // 内容类每窗口 1 次
    assert_eq!(moderate(&app, &m, t, "pin").await.status, StatusCode::OK);
    assert_eq!(
        code(&moderate(&app, &m, t, "unpin").await),
        (StatusCode::TOO_MANY_REQUESTS, Some("RATE_LIMITED"))
    );

    // 删帖：5 次之后再删即暂停管理权限（这一次不执行），并告警
    let mut posts = Vec::new();
    for _ in 0..6 {
        posts.push(reply_id(&app, &a, t).await);
    }
    for p in &posts[..5] {
        assert_eq!(
            delete_post(&app, &m, *p).await.status,
            StatusCode::NO_CONTENT
        );
    }
    let r = delete_post(&app, &m, posts[5]).await;
    assert_eq!(code(&r), (StatusCode::FORBIDDEN, Some("STAFF_SUSPENDED")));
    let status: i16 = sqlx::query_scalar("SELECT status FROM posts WHERE id = $1")
        .bind(posts[5])
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(status, 0);
    let alert: String = sqlx::query_scalar(
        "SELECT payload->>'text' FROM outbox WHERE kind = 'staff_alert' AND payload->>'event' = 'staff.anomaly'",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert!(alert.contains(&format!("#{m_id}")), "{alert}");
    // 暂停后任何管理操作都拒绝，也看不到隐藏内容；/me 显示已暂停
    let r = moderate(&app, &m, t, "lock").await;
    assert_eq!(code(&r).1, Some("STAFF_SUSPENDED"));
    let me = call(&app, "GET", "/api/v1/me", None, &m).await.body;
    assert_eq!(me["staff"]["suspended"], true);
    assert_eq!(me["staff"]["board_permissions"], json!([]));
    // 再触发不会重复记录
    delete_post(&app, &m, posts[5]).await;
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM staff_suspensions")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(n, 1);

    // 站长恢复后，之前的删帖不再计入
    let sid: i64 = sqlx::query_scalar("SELECT id FROM staff_suspensions")
        .fetch_one(&db)
        .await
        .unwrap();
    let (o_id, _) = member(&app, &db).await;
    staff::reinstate(&db, sid, o_id).await.unwrap().unwrap();
    assert_eq!(
        delete_post(&app, &m, posts[5]).await.status,
        StatusCode::NO_CONTENT
    );
}

// ---- 站长后台 ----

fn admin_call<'a>(
    app: &'a Router,
    method: &'a str,
    uri: &'a str,
    body: Option<Value>,
    cookies: &'a [String],
) -> impl std::future::Future<Output = Res> + 'a {
    call_from(app, ADMIN_ORIGIN, method, uri, body, cookies)
}

fn admin_url() -> Url {
    Url::parse(ADMIN_ORIGIN).unwrap()
}

/// 站长绑定第一个后台 Passkey：邮箱验证码 → 注册。返回软件 Passkey 与后台会话 Cookie。
async fn enroll(app: &Router, db: &PgPool, forum: &[String]) -> (SoftAuth, Vec<String>, Res) {
    let r = admin_call(app, "POST", "/api/v1/admin/enroll/code", None, forum).await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{:?}", r.body);
    let code: String = sqlx::query_scalar(
        "SELECT payload->>'code' FROM outbox WHERE kind = 'email.admin_code' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(db)
    .await
    .unwrap();
    let r = admin_call(
        app,
        "POST",
        "/api/v1/admin/enroll/verify",
        Some(json!({"code": code})),
        forum,
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{:?}", r.body);
    let mut auth = soft_passkey();
    let finish = register(app, &mut auth, forum).await;
    assert_eq!(finish.status, StatusCode::CREATED, "{:?}", finish.body);
    let cookies = jar(&finish.cookies);
    (auth, cookies, finish)
}

async fn register(app: &Router, auth: &mut SoftAuth, cookies: &[String]) -> Res {
    let r = admin_call(
        app,
        "POST",
        "/api/v1/admin/passkeys/register/start",
        None,
        cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    let ccr: CreationChallengeResponse = serde_json::from_value(r.body["options"].clone()).unwrap();
    let cred = auth.do_registration(admin_url(), ccr).unwrap();
    admin_call(
        app,
        "POST",
        "/api/v1/admin/passkeys/register/finish",
        Some(json!({"credential": cred})),
        cookies,
    )
    .await
}

/// 用后台 Passkey 完成一次断言（登录或再验证）。
async fn assertion(app: &Router, auth: &mut SoftAuth, kind: &str, cookies: &[String]) -> Res {
    let r = admin_call(
        app,
        "POST",
        &format!("/api/v1/admin/{kind}/start"),
        None,
        cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    let rcr: RequestChallengeResponse = serde_json::from_value(r.body["options"].clone()).unwrap();
    let cred = auth.do_authentication(admin_url(), rcr).unwrap();
    admin_call(
        app,
        "POST",
        &format!("/api/v1/admin/{kind}/finish"),
        Some(json!({"credential": cred})),
        cookies,
    )
    .await
}

/// 改 Valkey 里的后台会话字段（模拟时间流逝）。
async fn age_session(ns: &str, cookies: &[String], field: &str, secs_ago: i64) {
    let token = cookies
        .iter()
        .find_map(|c| c.strip_prefix("asid="))
        .unwrap();
    let key = format!(
        "{ns}admin:sess:{}",
        hex::encode(Sha256::digest(token.as_bytes()))
    );
    let kv = forum_kv::pool(&std::env::var("VALKEY_URL").unwrap()).unwrap();
    let mut s: Value =
        serde_json::from_str(&forum_kv::ephemeral::get(&kv, &key).await.unwrap().unwrap()).unwrap();
    s[field] = json!(time::OffsetDateTime::now_utc().unix_timestamp() - secs_ago);
    forum_kv::ephemeral::put(
        &kv,
        &key,
        &s.to_string(),
        std::time::Duration::from_secs(3600),
    )
    .await
    .unwrap();
}

/// 站长：论坛会话 + 后台 Passkey + 后台会话。
struct Owner {
    id: i64,
    forum: Vec<String>,
    admin: Vec<String>,
    auth: SoftAuth,
}

async fn owner(app: &Router, db: &PgPool) -> Owner {
    let (id, forum) = member(app, db).await;
    grant(db, id, staff::ROLE_OWNER, None).await;
    let (auth, admin, _) = enroll(app, db, &forum).await;
    Owner {
        id,
        forum,
        admin,
        auth,
    }
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn admin_backend_enrollment_login_and_session_rules(db: PgPool) {
    let ns = namespace();
    let Some(state) = state_with(db.clone(), wallet_state(), &ns) else {
        return;
    };
    let app = router_of(state);
    let (id, forum) = member(&app, &db).await;
    grant(&db, id, staff::ROLE_OWNER, None).await;

    // 论坛会话 Cookie 是 host-only（ADR 0012）
    let r = common::login(&app, &db, &unique_email(), &[]).await;
    assert!(r.cookies.iter().any(|c| c.starts_with("sid=")));
    assert!(
        r.cookies
            .iter()
            .all(|c| !c.to_ascii_lowercase().contains("domain")),
        "{:?}",
        r.cookies
    );

    // 未绑定：不能登录，不能跳过邮箱直接注册
    let r = admin_call(&app, "POST", "/api/v1/admin/login/start", None, &forum).await;
    assert_eq!(code(&r).1, Some("REVERIFY_REQUIRED"));
    let r = admin_call(
        &app,
        "POST",
        "/api/v1/admin/passkeys/register/start",
        None,
        &forum,
    )
    .await;
    assert_eq!(code(&r).1, Some("REVERIFY_REQUIRED"));
    // 登录验证码不能代替绑定验证码
    let r = admin_call(&app, "POST", "/api/v1/admin/enroll/code", None, &forum).await;
    assert_eq!(r.status, StatusCode::ACCEPTED);
    let login_code = latest_code(&db).await;
    let r = admin_call(
        &app,
        "POST",
        "/api/v1/admin/enroll/verify",
        Some(json!({"code": login_code})),
        &forum,
    )
    .await;
    assert_eq!(code(&r).1, Some("INVALID_CODE"));
    clear_resend_admin(&db, id).await;

    let (mut auth, admin, finish) = enroll(&app, &db, &forum).await;
    // 后台 Cookie：host-only、Strict、HttpOnly、Secure、限定路径
    let set = finish
        .cookies
        .iter()
        .find(|c| c.starts_with("asid="))
        .unwrap();
    for attr in [
        "HttpOnly",
        "SameSite=Strict",
        "Secure",
        "Path=/api/v1/admin",
    ] {
        assert!(set.contains(attr), "{set}");
    }
    assert!(!set.to_ascii_lowercase().contains("domain"), "{set}");
    let alert: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox WHERE kind = 'staff_alert' AND payload->>'event' = 'admin.passkey_add'",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(alert, 1);

    // 已有后台 Passkey：邮箱这条路关闭
    let r = admin_call(&app, "POST", "/api/v1/admin/enroll/code", None, &forum).await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    let r = admin_call(
        &app,
        "POST",
        "/api/v1/admin/passkeys/register/start",
        None,
        &forum,
    )
    .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);

    // 后台会话只认后台来源，GET 也要带
    let r = admin_call(&app, "GET", "/api/v1/admin/session", None, &admin).await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert_eq!(r.body["user_id"], id);
    for origin in [common::ORIGIN, WALLET_ORIGIN] {
        let r = call_from(&app, origin, "GET", "/api/v1/admin/session", None, &admin).await;
        assert_eq!(code(&r).1, Some("ORIGIN_REJECTED"), "{origin}");
    }
    let req = Request::get("/api/v1/admin/session")
        .header(header::COOKIE, admin.join("; "))
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    // 论坛会话不是后台会话
    let r = admin_call(&app, "GET", "/api/v1/admin/session", None, &forum).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);

    // 再加一个 Passkey 需要后台会话 + 5 分钟内再验证
    age_session(&ns, &admin, "reverified_at", 301).await;
    let r = admin_call(
        &app,
        "POST",
        "/api/v1/admin/passkeys/register/start",
        None,
        &admin,
    )
    .await;
    assert_eq!(code(&r).1, Some("ADMIN_REVERIFY_REQUIRED"));
    let r = assertion(&app, &mut auth, "reverify", &admin).await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert!(r.body["reverified_until"].as_i64().is_some());
    let mut second = soft_passkey();
    let r = register(&app, &mut second, &admin).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    let list = admin_call(&app, "GET", "/api/v1/admin/passkeys", None, &admin).await;
    assert_eq!(list.body["passkeys"].as_array().unwrap().len(), 2);

    // 闲置 2 小时、最长 12 小时
    age_session(&ns, &admin, "last_seen_at", 2 * 3600 + 1).await;
    let r = admin_call(&app, "GET", "/api/v1/admin/session", None, &admin).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    let r = assertion(&app, &mut second, "login", &forum).await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    let admin = jar(&r.cookies);
    age_session(&ns, &admin, "created_at", 12 * 3600 + 1).await;
    let r = admin_call(&app, "GET", "/api/v1/admin/session", None, &admin).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);

    // 退出；不再是站长时会话立即失效
    let admin = jar(&assertion(&app, &mut auth, "login", &forum).await.cookies);
    let r = admin_call(&app, "POST", "/api/v1/admin/logout", None, &admin).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert_eq!(
        admin_call(&app, "GET", "/api/v1/admin/session", None, &admin)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    let admin = jar(&assertion(&app, &mut auth, "login", &forum).await.cookies);
    sqlx::query("DELETE FROM staff_roles WHERE user_id = $1")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(
        admin_call(&app, "GET", "/api/v1/admin/session", None, &admin)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
}

/// 清掉后台验证码的 60 秒重发间隔。
#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn first_admin_passkey_email_marker_is_single_use(db: PgPool) {
    let ns = namespace();
    let Some(state) = state_with(db.clone(), wallet_state(), &ns) else {
        return;
    };
    let app = router_of(state);
    let (id, forum) = member(&app, &db).await;
    grant(&db, id, staff::ROLE_OWNER, None).await;
    let r = admin_call(&app, "POST", "/api/v1/admin/enroll/code", None, &forum).await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{:?}", r.body);
    let email_code: String = sqlx::query_scalar(
        "SELECT payload->>'code' FROM outbox WHERE kind = 'email.admin_code' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    let r = admin_call(
        &app,
        "POST",
        "/api/v1/admin/enroll/verify",
        Some(json!({"code": email_code})),
        &forum,
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{:?}", r.body);

    // 同一站长的另一个论坛会话用不了这个标记
    let email: String = sqlx::query_scalar("SELECT email FROM users WHERE id = $1")
        .bind(id)
        .fetch_one(&db)
        .await
        .unwrap();
    common::clear_resend(&email).await;
    let other = jar(&common::login(&app, &db, &email, &[]).await.cookies);
    let r = admin_call(
        &app,
        "POST",
        "/api/v1/admin/passkeys/register/start",
        None,
        &other,
    )
    .await;
    assert_eq!(code(&r).1, Some("REVERIFY_REQUIRED"));

    // 并发的两次绑定：第二个请求读到的会话数据里标记还在（复原第一个请求完成前的会话），
    // 第一个的 Passkey 也还没提交（删掉它）——标记已在 Valkey 里被原子地用掉，第二个仍被拒绝。
    let kv = forum_kv::pool(&std::env::var("VALKEY_URL").unwrap()).unwrap();
    let key = common::session_key(&forum);
    let before = forum_kv::ephemeral::get(&kv, &key).await.unwrap().unwrap();
    let mut auth = soft_passkey();
    let r = register(&app, &mut auth, &forum).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    sqlx::query("DELETE FROM passkeys WHERE user_id = $1 AND purpose = $2")
        .bind(id)
        .bind(staff::PURPOSE_ADMIN)
        .execute(&db)
        .await
        .unwrap();
    forum_kv::ephemeral::put(&kv, &key, &before, std::time::Duration::from_secs(600))
        .await
        .unwrap();
    let r = admin_call(
        &app,
        "POST",
        "/api/v1/admin/passkeys/register/start",
        None,
        &forum,
    )
    .await;
    assert_eq!(code(&r).1, Some("REVERIFY_REQUIRED"));
    assert!(
        forum_kv::ephemeral::get(
            &kv,
            &forum_api::routes::admin_auth::enroll_marker_key(&ns, id)
        )
        .await
        .unwrap()
        .is_none()
    );
}

async fn clear_resend_admin(db: &PgPool, user_id: i64) {
    let email: String = sqlx::query_scalar("SELECT email FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(db)
        .await
        .unwrap();
    clear_resend(&format!("admin-enroll\0{email}")).await;
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn admin_backend_is_owner_only_and_cors_scoped(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let (a_id, a) = member(&app, &db).await;
    grant(&db, a_id, staff::ROLE_ADMIN, None).await;
    let o = owner(&app, &db).await;

    // 管理员进不了后台，也就无法任免
    let r = admin_call(&app, "POST", "/api/v1/admin/login/start", None, &a).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let r = admin_call(&app, "POST", "/api/v1/admin/enroll/code", None, &a).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let appoint = json!({"user_id": a_id, "role": "moderator", "forum_id": null});
    let r = admin_call(
        &app,
        "POST",
        "/api/v1/admin/staff",
        Some(appoint.clone()),
        &a,
    )
    .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    // 拿着站长的后台 Cookie 从论坛页面发请求也不行
    let r = call(&app, "POST", "/api/v1/admin/staff", Some(appoint), &o.admin).await;
    assert_eq!(code(&r).1, Some("ORIGIN_REJECTED"));

    // CORS：后台来源只在后台接口上放行；钱包来源在后台接口上没有 CORS
    let preflight = |origin: &'static str, path: &'static str| {
        let app = app.clone();
        async move {
            let req = Request::builder()
                .method("OPTIONS")
                .uri(path)
                .header(header::ORIGIN, origin)
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                .body(Body::empty())
                .unwrap();
            app.oneshot(req)
                .await
                .unwrap()
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .map(|v| v.to_str().unwrap().to_owned())
        }
    };
    assert_eq!(
        preflight(ADMIN_ORIGIN, "/api/v1/admin/staff")
            .await
            .as_deref(),
        Some(ADMIN_ORIGIN)
    );
    assert_eq!(preflight(WALLET_ORIGIN, "/api/v1/admin/staff").await, None);
    assert_eq!(
        preflight(ADMIN_ORIGIN, "/api/v1/wallet/account").await,
        None
    );
    assert_eq!(
        preflight(WALLET_ORIGIN, "/api/v1/wallet/account")
            .await
            .as_deref(),
        Some(WALLET_ORIGIN)
    );
    // 后台来源不能调论坛写接口
    let r = call_from(
        &app,
        ADMIN_ORIGIN,
        "POST",
        "/api/v1/auth/logout",
        None,
        &o.forum,
    )
    .await;
    assert_eq!(code(&r).1, Some("ORIGIN_REJECTED"));
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn owner_manages_staff_settings_and_suspensions(db: PgPool) {
    let ns = namespace();
    let Some(state) = state_with(db.clone(), wallet_state(), &ns) else {
        return;
    };
    let app = router_of(state);
    let f = forum(&db, "general").await;
    let (u, uc) = member(&app, &db).await;
    let mut o = owner(&app, &db).await;

    // 任命版主（刚登录算再验证过）；参数不对、重复任命拒绝
    let appoint = |body: Value| {
        let (app, c) = (app.clone(), o.admin.clone());
        async move { admin_call(&app, "POST", "/api/v1/admin/staff", Some(body), &c).await }
    };
    assert_eq!(
        appoint(json!({"user_id": u, "role": "owner"})).await.status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        appoint(json!({"user_id": u, "role": "moderator"}))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    let r = appoint(json!({"user_id": u, "role": "moderator", "forum_id": f})).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    let assignment = r.body["id"].as_i64().unwrap();
    assert_eq!(
        appoint(json!({"user_id": u, "role": "moderator", "forum_id": f}))
            .await
            .status,
        StatusCode::CONFLICT
    );
    let me = call(&app, "GET", "/api/v1/me", None, &uc).await.body;
    assert_eq!(me["staff"]["boards"], json!([f]));
    let list = admin_call(&app, "GET", "/api/v1/admin/staff", None, &o.admin).await;
    assert_eq!(list.body["staff"].as_array().unwrap().len(), 2);

    // 超过 5 分钟：敏感操作要先再验证
    age_session(&ns, &o.admin, "reverified_at", 301).await;
    let r = appoint(json!({"user_id": u, "role": "admin"})).await;
    assert_eq!(code(&r).1, Some("ADMIN_REVERIFY_REQUIRED"));
    let path = format!("/api/v1/admin/staff/{assignment}?reason=调岗");
    assert_eq!(
        code(&admin_call(&app, "DELETE", &path, None, &o.admin).await).1,
        Some("ADMIN_REVERIFY_REQUIRED")
    );
    // 只读不需要
    assert_eq!(
        admin_call(
            &app,
            "GET",
            "/api/v1/admin/settings/moderation",
            None,
            &o.admin
        )
        .await
        .status,
        StatusCode::OK
    );
    assertion(&app, &mut o.auth, "reverify", &o.admin).await;
    assert_eq!(
        admin_call(&app, "DELETE", &path, None, &o.admin)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert!(call(&app, "GET", "/api/v1/me", None, &uc).await.body["staff"].is_null());
    // 站长自己的任命不能经接口撤销
    let own: i64 = sqlx::query_scalar("SELECT id FROM staff_roles WHERE user_id = $1")
        .bind(o.id)
        .fetch_one(&db)
        .await
        .unwrap();
    let r = admin_call(
        &app,
        "DELETE",
        &format!("/api/v1/admin/staff/{own}?reason=x"),
        None,
        &o.admin,
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);

    // 阈值：越界拒绝，合法的生效
    let put = |body: Value| {
        let (app, c) = (app.clone(), o.admin.clone());
        async move {
            admin_call(
                &app,
                "PUT",
                "/api/v1/admin/settings/moderation",
                Some(body),
                &c,
            )
            .await
        }
    };
    assert_eq!(
        put(json!({"anomaly_deletes": 1})).await.status,
        StatusCode::BAD_REQUEST
    );
    let r = put(json!({"anomaly_deletes": 7, "rate_delete": 99})).await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert_eq!(r.body["settings"]["anomaly_deletes"], 7);
    assert_eq!(r.body["settings"]["rate_content"], 60);
    assert_eq!(r.body["min"]["anomaly_deletes"], 5);
    let saved = forum_db::settings::moderation(&db).await.unwrap();
    assert_eq!((saved.anomaly_deletes, saved.rate_delete), (7, 99));

    // 恢复被暂停的管理权限
    let sid = staff::suspend(&db, u, "anomaly", &json!({}))
        .await
        .unwrap()
        .unwrap();
    let r = admin_call(&app, "GET", "/api/v1/admin/suspensions", None, &o.admin).await;
    assert_eq!(r.body["suspensions"][0]["user_id"], u);
    let path = format!("/api/v1/admin/suspensions/{sid}/reinstate?reason=已核实");
    assert_eq!(
        admin_call(&app, "POST", &path, None, &o.admin).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        admin_call(&app, "POST", &path, None, &o.admin).await.status,
        StatusCode::NOT_FOUND
    );

    let actions: Vec<String> =
        sqlx::query_scalar("SELECT action FROM admin_audit_logs WHERE actor_id = $1 ORDER BY id")
            .bind(o.id)
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(
        actions,
        [
            "admin.passkey_add",
            "staff.appoint",
            "staff.remove",
            "settings.moderation",
            "staff.reinstate"
        ]
    );
    let alerts: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox WHERE kind = 'staff_alert'")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(alerts, 5);
    let r = admin_call(
        &app,
        "GET",
        &format!("/api/v1/admin/audit?actor_id={}", o.id),
        None,
        &o.admin,
    )
    .await;
    assert_eq!(r.body["entries"].as_array().unwrap().len(), 5);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn batch_revoke_preview_and_execute(db: PgPool) {
    let ns = namespace();
    let Some(state) = state_with(db.clone(), wallet_state(), &ns) else {
        return;
    };
    let app = router_of(state);
    let f = forum(&db, "general").await;
    let (u, uc) = member(&app, &db).await;
    let (m_id, m) = member(&app, &db).await;
    let (a_id, a) = member(&app, &db).await;
    grant(&db, m_id, staff::ROLE_MODERATOR, Some(f)).await;
    grant(&db, a_id, staff::ROLE_ADMIN, None).await;
    let mut o = owner(&app, &db).await;
    let t = topic(&app, &uc, "general").await;
    let p = reply_id(&app, &uc, t).await;
    let p2 = reply_id(&app, &uc, t).await;
    let from = time::OffsetDateTime::now_utc() - time::Duration::minutes(5);

    // 被盗的版主账号：置顶、隐藏、锁定、删帖、编辑不了（没权限）、禁言
    for action in ["pin", "hide", "lock"] {
        assert_eq!(moderate(&app, &m, t, action).await.status, StatusCode::OK);
    }
    assert_eq!(
        delete_post(&app, &m, p).await.status,
        StatusCode::NO_CONTENT
    );
    let r = sanction(
        &app,
        &m,
        u,
        json!({"kind": "mute", "forum_id": f, "duration_secs": DAY, "reason": "x"}),
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED);
    // 之后管理员已经手动解锁：撤销「锁定」时不覆盖
    assert_eq!(moderate(&app, &a, t, "unlock").await.status, StatusCode::OK);
    // 管理员另一条操作不受影响
    assert_eq!(
        delete_post(&app, &a, p2).await.status,
        StatusCode::NO_CONTENT
    );
    let to = time::OffsetDateTime::now_utc() + time::Duration::minutes(1);

    let body = |execute: bool| {
        json!({
            "actor_id": m_id, "from": from.format(&Rfc3339).unwrap(), "to": to.format(&Rfc3339).unwrap(),
            "reason": "账号被盗", "execute": execute,
        })
    };
    let statuses = |r: &Res| -> Vec<(String, String)> {
        r.body["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| {
                (
                    i["action"].as_str().unwrap().to_owned(),
                    i["status"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    };
    let flags = || async {
        sqlx::query_scalar::<_, i16>("SELECT flags FROM topics WHERE id = $1")
            .bind(t)
            .fetch_one(&db)
            .await
            .unwrap()
    };
    let expected = [
        ("user.mute", "reverted"),
        ("post.delete", "reverted"),
        ("topic.lock", "superseded"),
        ("topic.hide", "reverted"),
        ("topic.pin", "reverted"),
    ];
    let expected: Vec<(String, String)> = expected
        .iter()
        .map(|(a, s)| ((*a).to_owned(), (*s).to_owned()))
        .collect();

    // 预览：什么都不改
    let r = admin_call(
        &app,
        "POST",
        "/api/v1/admin/revoke",
        Some(body(false)),
        &o.admin,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert_eq!(statuses(&r), expected);
    assert_eq!(r.body["executed"], false);
    assert_eq!(flags().await, 1 | 8);
    // 执行需要 5 分钟内再验证
    age_session(&ns, &o.admin, "reverified_at", 301).await;
    let r = admin_call(
        &app,
        "POST",
        "/api/v1/admin/revoke",
        Some(body(true)),
        &o.admin,
    )
    .await;
    assert_eq!(code(&r).1, Some("ADMIN_REVERIFY_REQUIRED"));
    assertion(&app, &mut o.auth, "reverify", &o.admin).await;
    let r = admin_call(
        &app,
        "POST",
        "/api/v1/admin/revoke",
        Some(body(true)),
        &o.admin,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert_eq!(statuses(&r), expected);
    assert_eq!(r.body["reverted"], 4);

    assert_eq!(flags().await, 0);
    let status: Vec<i16> =
        sqlx::query_scalar("SELECT status FROM posts WHERE id = ANY($1) ORDER BY id")
            .bind(vec![p, p2])
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(status, [0, 2]);
    let active: i64 = sqlx::query_scalar("SELECT count(*) FROM sanctions WHERE revoked_at IS NULL")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(active, 0);
    assert_eq!(reply(&app, &uc, t).await.status, StatusCode::CREATED);
    // 板块列表与主题页缓存已更新
    let board = call(&app, "GET", "/api/v1/forums/general/topics", None, &[])
        .await
        .body;
    assert_eq!(board["topics"][0]["id"], t);

    // 再执行一次：都已撤销
    let r = admin_call(
        &app,
        "POST",
        "/api/v1/admin/revoke",
        Some(body(true)),
        &o.admin,
    )
    .await;
    let again: Vec<String> = statuses(&r).into_iter().map(|(_, s)| s).collect();
    assert_eq!(
        again,
        [
            "already_reverted",
            "already_reverted",
            "superseded",
            "already_reverted",
            "already_reverted"
        ]
    );
    let reverts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM admin_audit_logs WHERE reverts IS NOT NULL")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(reverts, 4);
    let batches: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM admin_audit_logs WHERE action = 'admin.revoke_actions' AND actor_id = $1",
    )
    .bind(o.id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(batches, 2);

    // 时间段不合法
    let mut bad = body(false);
    bad["to"] = bad["from"].clone();
    let r = admin_call(&app, "POST", "/api/v1/admin/revoke", Some(bad), &o.admin).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn revert_restores_staff_edit(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    forum(&db, "general").await;
    let (_, uc) = member(&app, &db).await;
    let (a_id, a) = member(&app, &db).await;
    grant(&db, a_id, staff::ROLE_ADMIN, None).await;
    let o = owner(&app, &db).await;
    let t = topic(&app, &uc, "general").await;
    let first: i64 = sqlx::query_scalar("SELECT id FROM posts WHERE topic_id = $1")
        .bind(t)
        .fetch_one(&db)
        .await
        .unwrap();
    let from = time::OffsetDateTime::now_utc() - time::Duration::minutes(1);
    let r = call(
        &app,
        "PATCH",
        &format!("/api/v1/posts/{first}"),
        Some(json!({"html": "<p>篡改</p>", "source": doc("篡改"), "title": "被改的标题"})),
        &a,
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{:?}", r.body);
    let to = time::OffsetDateTime::now_utc() + time::Duration::minutes(1);
    let r = admin_call(
        &app,
        "POST",
        "/api/v1/admin/revoke",
        Some(json!({
            "actor_id": a_id, "from": from.format(&Rfc3339).unwrap(), "to": to.format(&Rfc3339).unwrap(),
            "reason": "恢复原文", "execute": true,
        })),
        &o.admin,
    )
    .await;
    assert_eq!(r.body["reverted"], 1, "{:?}", r.body);
    let page = call(&app, "GET", &format!("/api/v1/topics/{t}"), None, &[])
        .await
        .body;
    assert_eq!(page["topic"]["title"], "t");
    assert_eq!(page["posts"][0]["html"], "<p>x</p>");
    let src = call(
        &app,
        "GET",
        &format!("/api/v1/posts/{first}/source"),
        None,
        &uc,
    )
    .await;
    assert_eq!(src.body["source"], doc("x"));
}

/// 申诉中的案件（订单 `usdt` 整 USDT，争议发生在 `disputed_days_ago` 天前），没有复核人。
async fn appealed_case(db: &PgPool, buyer: i64, seller: i64, disputed_days_ago: i32) -> i64 {
    sqlx::query(
        "INSERT INTO chains (chain_id, name, escrow_address, config_address, token_address, token_symbol,
           token_decimals, start_block, safe_depth, finalized_depth)
         VALUES (421614, 'test', decode(repeat('44', 20), 'hex'), decode(repeat('c0', 20), 'hex'),
           decode(repeat('33', 20), 'hex'), 'USDT', 6, 0, 240, 4800)
         ON CONFLICT DO NOTHING",
    )
    .execute(db)
    .await
    .unwrap();
    sqlx::query_scalar(
        "WITH o AS (
           INSERT INTO escrow_orders (chain_id, contract, chain_order_id, mode, buyer, seller, buyer_id, seller_id,
             token, amount, instant, offer_hash, listing_hash, state, disputed_at, level, created_block, updated_block)
           VALUES (421614, decode(repeat('44', 20), 'hex'),
             (SELECT coalesce(max(chain_order_id), 0) + 7 FROM escrow_orders), 0,
             decode(repeat('b1', 20), 'hex'), decode(repeat('c1', 20), 'hex'), $1, $2,
             decode(repeat('33', 20), 'hex'), 45000000, false, decode(repeat('ab', 32), 'hex'),
             decode(repeat('ab', 32), 'hex'), 2, now() - make_interval(days => $3), 'safe', 1, 1)
           RETURNING id
         )
         INSERT INTO arbitration_cases (order_id, panel_size, evidence_until, status, appeal_by, appeal_reason)
         SELECT id, 1, now() - interval '1 day', 'appealed', $1, '不服' FROM o
         RETURNING id",
    )
    .bind(buyer)
    .bind(seller)
    .bind(disputed_days_ago)
    .fetch_one(db)
    .await
    .unwrap()
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn owner_lookups_and_arbitration_overview(db: PgPool) {
    let ns = namespace();
    let Some(state) = state_with(db.clone(), wallet_state(), &ns) else {
        return;
    };
    let app = router_of(state);
    let f = forum(&db, "general").await;
    let (u, _) = member(&app, &db).await;
    set_username(&db, u, &format!("lookup{u}")).await;
    let (seller, _) = member(&app, &db).await;
    let o = owner(&app, &db).await;

    // 任命用的查人与板块（后台页面不能跨源调论坛接口）
    let r = admin_call(
        &app,
        "GET",
        &format!("/api/v1/admin/users/search?q=lookup{u}"),
        None,
        &o.admin,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert_eq!(r.body[0]["id"], json!(u));
    let r = admin_call(&app, "GET", "/api/v1/admin/forums", None, &o.admin).await;
    assert!(
        r.body
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["id"] == json!(f))
    );

    // 仲裁总览：申诉后没有复核人 = 缺人；争议 28 天前发生 = 临近 30 天超时兜底
    let case = appealed_case(&db, u, seller, 28).await;
    let r = admin_call(
        &app,
        "GET",
        "/api/v1/admin/arbitration?days=30",
        None,
        &o.admin,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    let c = r.body["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["case_id"] == json!(case))
        .cloned()
        .unwrap();
    assert_eq!(c["status"], "appealed");
    assert_eq!(c["unstaffed"], true);
    assert_eq!(c["timeout_soon"], true);
    assert_eq!(r.body["workload"]["days"], 30);
    assert_eq!(
        admin_call(
            &app,
            "GET",
            "/api/v1/admin/arbitration?days=0",
            None,
            &o.admin
        )
        .await
        .status,
        StatusCode::BAD_REQUEST
    );
    let r = admin_call(
        &app,
        "GET",
        "/api/v1/admin/arbitration/execution",
        None,
        &o.admin,
    )
    .await;
    assert_eq!(r.body, json!([]));

    // 只有后台会话能看：论坛会话不行，论坛来源也不行
    for path in [
        "/api/v1/admin/arbitration",
        "/api/v1/admin/forums",
        "/api/v1/admin/users/search?q=a",
    ] {
        assert_eq!(
            admin_call(&app, "GET", path, None, &o.forum).await.status,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            code(&call(&app, "GET", path, None, &o.admin).await).1,
            Some("ORIGIN_REJECTED")
        );
    }
}
