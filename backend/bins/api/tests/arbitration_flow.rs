//! 仲裁：裁决 Passkey 登记（与钱包 Passkey 分开）、两人盲审 → 分歧 → 站长终裁 → 申诉 → 复核 → Safe 执行数据、
//! 叫停与恢复、challenge 绑定裁决内容（篡改即拒绝、断言不能重用）、聊天哈希链损坏时拒绝裁决、陈述与举证期、访问控制。
//! 需要 PostgreSQL（`DATABASE_URL`）与 Valkey（`VALKEY_URL`，未设置则跳过）。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

mod common;

use alloy::primitives::{Address, Bytes, U256};
use alloy::sol_types::SolCall;
use axum::Router;
use axum::http::StatusCode;
use common::{
    ESCROW, ORIGIN, RULING_RP_ID, Res, SoftAuth, WALLET_ORIGIN, build, call, call_from,
    set_username, soft_passkey, user,
};
use forum_auth::passkey::webauthn::{RequestChallengeResponse, Url};
use forum_chain::bindings::Escrow;
use forum_db::PgPool;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use time::{Duration, OffsetDateTime};

const OWNER: i16 = 1;
const ADMIN: i16 = 3;

async fn staff(db: &PgPool, user_id: i64, role: i16) {
    sqlx::query("INSERT INTO staff_roles (user_id, role) VALUES ($1, $2)")
        .bind(user_id)
        .bind(role)
        .execute(db)
        .await
        .unwrap();
}

/// 争议中的订单（`usdt` 整 USDT，合约为测试配置的 ESCROW），并像 indexer 一样同步出案件。返回 `(订单, 案件)`。
async fn disputed_order(db: &PgPool, buyer: i64, seller: i64, usdt: i64) -> (i64, i64) {
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
    let order: i64 = sqlx::query_scalar(
        "INSERT INTO escrow_orders (chain_id, contract, chain_order_id, mode, buyer, seller, buyer_id, seller_id,
           token, amount, instant, offer_hash, listing_hash, state, disputed_at, level, created_block, updated_block)
         VALUES (421614, decode(repeat('44', 20), 'hex'),
           (SELECT coalesce(max(chain_order_id), 0) + 7 FROM escrow_orders), 0,
           decode(repeat('b1', 20), 'hex'), decode(repeat('c1', 20), 'hex'), $1, $2,
           decode(repeat('33', 20), 'hex'), $3::bigint * 1000000, false, decode(repeat('ab', 32), 'hex'),
           decode(repeat('ab', 32), 'hex'), 2, now(), 'safe', 1, 1)
         RETURNING id",
    )
    .bind(buyer)
    .bind(seller)
    .bind(usdt)
    .fetch_one(db)
    .await
    .unwrap();
    let mut tx = db.begin().await.unwrap();
    let synced = forum_db::arbitration::sync_order(&mut tx, order)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let Some(forum_db::arbitration::Synced::Opened(case)) = synced else {
        panic!("no case: {synced:?}")
    };
    (order, case)
}

async fn end_evidence(db: &PgPool, case_id: i64) {
    sqlx::query(
        "UPDATE arbitration_cases SET evidence_until = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(case_id)
    .execute(db)
    .await
    .unwrap();
}

async fn tick(db: &PgPool) {
    forum_db::arbitration::tick(db, OffsetDateTime::now_utc(), Duration::hours(48))
        .await
        .unwrap();
}

async fn seated(db: &PgPool, case_id: i64, role: &str) -> Vec<i64> {
    forum_db::arbitration::assignments(db, case_id)
        .await
        .unwrap()
        .into_iter()
        .filter(|a| a.role == role && a.counts())
        .map(|a| a.arbitrator_id)
        .collect()
}

/// 登记裁决 Passkey：邮箱验证码 → 注册（软件 Passkey，论坛来源）。
async fn enroll(app: &Router, db: &PgPool, user_id: i64, cookies: &[String]) -> SoftAuth {
    set_username(db, user_id, &format!("staff{user_id}")).await;
    let r = call(
        app,
        "POST",
        "/api/v1/arbitration/passkey/reverify/start",
        None,
        cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{:?}", r.body);
    let code: String = sqlx::query_scalar(
        "SELECT payload->>'code' FROM outbox WHERE kind = 'email.ruling_passkey_code' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(db)
    .await
    .unwrap();
    let r = call(
        app,
        "POST",
        "/api/v1/arbitration/passkey/reverify/verify",
        Some(json!({"code": code})),
        cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{:?}", r.body);
    let r = call(
        app,
        "POST",
        "/api/v1/arbitration/passkey/register/start",
        None,
        cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert_eq!(r.body["options"]["publicKey"]["rp"]["id"], RULING_RP_ID);
    let mut auth = soft_passkey();
    let cred = auth
        .do_registration(
            Url::parse(ORIGIN).unwrap(),
            serde_json::from_value(r.body["options"].clone()).unwrap(),
        )
        .unwrap();
    let r = call(
        app,
        "POST",
        "/api/v1/arbitration/passkey/register/finish",
        Some(json!({"credential": cred})),
        cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    auth
}

fn ruling(kind: &str, bps: u32, reason: &str) -> Value {
    json!({"kind": kind, "to_buyer_bps": bps, "bond_to_buyer_bps": 0, "reason": reason})
}

/// 开始确认并用软件 Passkey 签出断言；返回 `(options 响应, 断言)`。
async fn sign(
    app: &Router,
    auth: &mut SoftAuth,
    cookies: &[String],
    case: i64,
    body: &Value,
) -> (Res, Value) {
    let r = call(
        app,
        "POST",
        &format!("/api/v1/arbitration/cases/{case}/ruling/options"),
        Some(body.clone()),
        cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    let rcr: RequestChallengeResponse = serde_json::from_value(r.body["options"].clone()).unwrap();
    let cred = auth
        .do_authentication(Url::parse(ORIGIN).unwrap(), rcr)
        .unwrap();
    (r, serde_json::to_value(cred).unwrap())
}

async fn submit(app: &Router, cookies: &[String], case: i64, body: &Value, cred: &Value) -> Res {
    let mut b = body.clone();
    b["credential"] = cred.clone();
    call(
        app,
        "POST",
        &format!("/api/v1/arbitration/cases/{case}/rulings"),
        Some(b),
        cookies,
    )
    .await
}

/// 确认并提交一份裁决。
async fn rule(
    app: &Router,
    auth: &mut SoftAuth,
    cookies: &[String],
    case: i64,
    body: Value,
) -> Res {
    let (_, cred) = sign(app, auth, cookies, case, &body).await;
    submit(app, cookies, case, &body, &cred).await
}

async fn get_case(app: &Router, cookies: &[String], case: i64) -> Res {
    call(
        app,
        "GET",
        &format!("/api/v1/arbitration/cases/{case}"),
        None,
        cookies,
    )
    .await
}

fn ruling_ids(detail: &Value) -> Vec<Value> {
    detail["rulings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["arbitrator_id"].clone())
        .collect()
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn ruling_passkeys_are_separate_from_wallet_passkeys(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let (plain, plain_c) = user(&app, &db).await;
    set_username(&db, plain, "plain").await;
    // 不是站长或管理员：一律 403
    for (m, p) in [
        ("GET", "/api/v1/arbitration/passkeys"),
        ("POST", "/api/v1/arbitration/passkey/reverify/start"),
        ("POST", "/api/v1/arbitration/passkey/register/start"),
        ("GET", "/api/v1/arbitration/queue"),
    ] {
        let r = call(&app, m, p, None, &plain_c).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN, "{p}");
    }
    let (admin, c) = user(&app, &db).await;
    staff(&db, admin, ADMIN).await;
    set_username(&db, admin, "admin").await;
    // 钱包页面来源不能发起裁决 Passkey 仪式
    let r = call_from(
        &app,
        WALLET_ORIGIN,
        "POST",
        "/api/v1/arbitration/passkey/register/start",
        None,
        &c,
    )
    .await;
    assert_eq!(r.body["code"], "ORIGIN_REJECTED");
    // 第一个需要专门的邮箱验证
    let r = call(
        &app,
        "POST",
        "/api/v1/arbitration/passkey/register/start",
        None,
        &c,
    )
    .await;
    assert_eq!(r.body["code"], "REVERIFY_REQUIRED");
    let _auth = enroll(&app, &db, admin, &c).await;
    let r = call(&app, "GET", "/api/v1/arbitration/passkeys", None, &c).await;
    assert_eq!(r.body["passkeys"].as_array().unwrap().len(), 1);
    let purpose: i16 = sqlx::query_scalar("SELECT purpose FROM passkeys WHERE user_id = $1")
        .bind(admin)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(purpose, forum_db::arbitration::PURPOSE_RULING);
    // 钱包侧完全看不到这组凭证
    let r = call(&app, "GET", "/api/v1/wallet", None, &c).await;
    assert_eq!(r.body["passkeys"], json!([]));
    let r = call_from(
        &app,
        WALLET_ORIGIN,
        "POST",
        "/api/v1/wallet/passkey/assert/start",
        None,
        &c,
    )
    .await;
    assert_eq!(r.body["code"], "WALLET_REQUIRED");
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn blind_two_person_flow_owner_appeal_and_execution(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let (buyer, buyer_c) = user(&app, &db).await;
    let (seller, seller_c) = user(&app, &db).await;
    let (owner, owner_c) = user(&app, &db).await;
    let (_, outsider_c) = user(&app, &db).await;
    staff(&db, owner, OWNER).await;
    let mut admins = Vec::new();
    for _ in 0..3 {
        let (id, c) = user(&app, &db).await;
        staff(&db, id, ADMIN).await;
        admins.push((id, c));
    }
    let (order, case) = disputed_order(&db, buyer, seller, 200).await;
    end_evidence(&db, case).await;
    tick(&db).await;
    let panel = seated(&db, case, "panel").await;
    assert_eq!(panel.len(), 2);
    let cookies = |id: i64| admins.iter().find(|(a, _)| *a == id).unwrap().1.clone();
    let (p1, p2) = (panel[0], panel[1]);
    let (c1, c2) = (cookies(p1), cookies(p2));
    let (p3, c3) = admins
        .iter()
        .find(|(a, _)| !panel.contains(a))
        .cloned()
        .unwrap();
    let mut a1 = enroll(&app, &db, p1, &c1).await;
    let mut a2 = enroll(&app, &db, p2, &c2).await;
    let mut a3 = enroll(&app, &db, p3, &c3).await;
    let mut ao = enroll(&app, &db, owner, &owner_c).await;

    // 外人、没被分配的管理员看不到案件，也看不到订单聊天
    for c in [&outsider_c, &c3] {
        assert_eq!(get_case(&app, c, case).await.status, StatusCode::NOT_FOUND);
        let r = call(
            &app,
            "GET",
            &format!("/api/v1/orders/{order}/messages"),
            None,
            c,
        )
        .await;
        assert_eq!(r.status, StatusCode::NOT_FOUND);
    }
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/orders/{order}/messages"),
        None,
        &c1,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    let d = get_case(&app, &buyer_c, case).await.body;
    assert_eq!(d["actions"]["seat"], "buyer");
    assert_eq!(
        (
            d["rulings"].clone(),
            d["assignments"].clone(),
            d["chat"].clone()
        ),
        (json!([]), json!([]), Value::Null)
    );

    // 第一人裁决：等另一人
    let r = rule(
        &app,
        &mut a1,
        &c1,
        case,
        ruling("panel", 10_000, "卖家未发货"),
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    assert_eq!(r.body["outcome"], "waiting");
    // 盲审：另一人看不到第一人的裁决、分配，第一人也只看到自己的
    let d = get_case(&app, &c2, case).await.body;
    assert_eq!(d["rulings"], json!([]));
    assert_eq!(d["assignments"].as_array().unwrap().len(), 1);
    assert_eq!(d["assignments"][0]["arbitrator_id"], p2);
    assert_eq!(d["actions"]["rule"], "panel");
    assert!(d["chat"]["verification"]["ok"].as_bool().unwrap());
    let d = get_case(&app, &c1, case).await.body;
    assert_eq!(ruling_ids(&d), [json!(p1)]);
    assert_eq!(d["actions"]["rule"], Value::Null);
    // 已提交的人不能再交，也不能冒充站长
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/arbitration/cases/{case}/ruling/options"),
        Some(ruling("panel", 0, "x")),
        &c1,
    )
    .await;
    assert_eq!(r.body["code"], "FORBIDDEN");
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/arbitration/cases/{case}/ruling/options"),
        Some(ruling("owner", 0, "x")),
        &c1,
    )
    .await;
    assert_eq!(r.body["code"], "FORBIDDEN");

    // 第二人意见不同：转站长终裁，两人此时都能看到双方裁决
    let r = rule(
        &app,
        &mut a2,
        &c2,
        case,
        ruling("panel", 5_000, "双方都有责任"),
    )
    .await;
    assert_eq!(r.body["outcome"], "disagreed", "{:?}", r.body);
    assert_eq!(r.body["status"], "owner_review");
    let d = get_case(&app, &c1, case).await.body;
    assert_eq!(d["rulings"].as_array().unwrap().len(), 2);
    // 公示前当事人什么都看不到
    assert_eq!(
        get_case(&app, &seller_c, case).await.body["rulings"],
        json!([])
    );

    let r = rule(
        &app,
        &mut ao,
        &owner_c,
        case,
        ruling("owner", 5_000, "证据显示部分交付"),
    )
    .await;
    assert_eq!(r.body["outcome"], "published", "{:?}", r.body);
    // 公示后当事人看到全部裁决与理由，但看不到是谁判的
    let d = get_case(&app, &buyer_c, case).await.body;
    assert_eq!(ruling_ids(&d), [Value::Null, Value::Null, Value::Null]);
    assert_eq!(d["rulings"][2]["effective"], true);
    assert_eq!(d["rulings"][2]["principal_to_buyer"], "100000000");
    assert_eq!(d["actions"]["appeal"], true);
    let public = call(
        &app,
        "GET",
        &format!("/api/v1/arbitration/public/{case}"),
        None,
        &[],
    )
    .await;
    assert_eq!(public.status, StatusCode::OK);
    assert_eq!(ruling_ids(&public.body).len(), 3);
    assert!(!public.body.to_string().contains("buyer_id"));

    // 申诉一次：卖家申诉后买家不能再申诉；外人不能申诉
    let appeal = |c: Vec<String>| {
        let app = app.clone();
        async move {
            call(
                &app,
                "POST",
                &format!("/api/v1/arbitration/cases/{case}/appeal"),
                Some(json!({"reason": "不服"})),
                &c,
            )
            .await
        }
    };
    assert_eq!(
        appeal(outsider_c.clone()).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        appeal(seller_c.clone()).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        appeal(buyer_c.clone()).await.body["code"],
        "CASE_STATE_INVALID"
    );

    // 申诉中叫停：站长不能借叫停直接改判（复核必须由没参与的人做），只能恢复
    let halt = format!("/api/v1/arbitration/cases/{case}/halt");
    let resume = format!("/api/v1/arbitration/cases/{case}/resume");
    let r = call(
        &app,
        "POST",
        &halt,
        Some(json!({"reason": "核查"})),
        &owner_c,
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{:?}", r.body);
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/arbitration/cases/{case}/ruling/options"),
        Some(ruling("owner", 0, "x")),
        &owner_c,
    )
    .await;
    assert_eq!(r.body["code"], "CASE_STATE_INVALID");
    // 当事人看得到叫停状态，看不到是谁叫停的
    let d = get_case(&app, &buyer_c, case).await.body;
    assert_eq!(
        (d["status"].clone(), d["halt"].clone()),
        (json!("halted"), Value::Null)
    );
    assert_eq!(
        call(&app, "POST", &resume, None, &owner_c).await.status,
        StatusCode::NO_CONTENT
    );

    // 复核人是唯一没参与过的管理员（站长作过终裁，也参与过）
    tick(&db).await;
    assert_eq!(seated(&db, case, "appeal").await, [p3]);
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/arbitration/cases/{case}/ruling/options"),
        Some(ruling("appeal", 0, "x")),
        &owner_c,
    )
    .await;
    assert_eq!(r.body["code"], "FORBIDDEN");
    let r = rule(
        &app,
        &mut a3,
        &c3,
        case,
        ruling("appeal", 7_500, "复核：卖家主要责任"),
    )
    .await;
    assert_eq!(r.body["outcome"], "final", "{:?}", r.body);
    assert_eq!(r.body["status"], "ready");

    // Safe 执行数据：calldata 解码回来就是生效裁决
    let d = get_case(&app, &c1, case).await.body;
    let ex = &d["execution"];
    assert_eq!(ex["to"].as_str().unwrap().to_lowercase(), ESCROW);
    let data: Bytes = ex["data"].as_str().unwrap().parse().unwrap();
    let call_back = Escrow::resolveCall::abi_decode(&data).unwrap();
    assert_eq!(
        call_back.orderId,
        U256::from(d["order"]["chain_order_id"].as_u64().unwrap())
    );
    assert_eq!(call_back.toBuyerBps, U256::from(7_500));
    assert_eq!(call_back.bondToBuyerBps, U256::ZERO);
    assert_eq!(ex["safe"], Address::repeat_byte(0x5a).to_checksum(None));
    assert_eq!(
        ex["transaction_builder"]["transactions"][0]["data"],
        ex["data"]
    );
    assert_eq!(ex["transaction_builder"]["chainId"], "421614");
    // 批量文件：仲裁 Safe 的签名人（站长、管理员）可取，其他人不行
    let r = call(&app, "GET", "/api/v1/arbitration/execution", None, &c3).await;
    assert_eq!(r.body[0]["cases"][0]["case_id"], case);
    assert_eq!(
        call(&app, "GET", "/api/v1/arbitration/execution", None, &buyer_c)
            .await
            .status,
        StatusCode::FORBIDDEN
    );

    // 站长叫停：没有执行数据；恢复后回到可执行
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/arbitration/cases/{case}/halt"),
        Some(json!({"reason": "复查证据"})),
        &c1,
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/arbitration/cases/{case}/halt"),
        Some(json!({"reason": "复查证据"})),
        &owner_c,
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{:?}", r.body);
    let d = get_case(&app, &c1, case).await.body;
    assert_eq!(
        (d["status"].clone(), d["execution"].clone()),
        (json!("halted"), Value::Null)
    );
    assert_eq!(
        call(&app, "GET", "/api/v1/arbitration/execution", None, &c3)
            .await
            .body,
        json!([])
    );
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/arbitration/cases/{case}/resume"),
        None,
        &owner_c,
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert_eq!(get_case(&app, &owner_c, case).await.body["status"], "ready");

    // 计酬统计只给站长
    let r = call(
        &app,
        "GET",
        "/api/v1/arbitration/workload?days=7",
        None,
        &owner_c,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.body["arbitrators"].as_array().unwrap().len(), 4);
    assert_eq!(
        call(&app, "GET", "/api/v1/arbitration/workload", None, &c1)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    let actions: Vec<String> = sqlx::query_scalar(
        "SELECT action FROM admin_audit_logs WHERE target = $1 AND action IN
           ('arbitration.ruling', 'arbitration.disagreed', 'arbitration.published', 'arbitration.appealed',
            'arbitration.halted', 'arbitration.resumed')
         ORDER BY id",
    )
    .bind(format!("arbitration_case:{case}"))
    .fetch_all(&db)
    .await
    .unwrap();
    assert_eq!(
        actions,
        [
            "arbitration.ruling",
            "arbitration.ruling",
            "arbitration.disagreed",
            "arbitration.ruling",
            "arbitration.published",
            "arbitration.appealed",
            "arbitration.halted",
            "arbitration.resumed",
            "arbitration.ruling",
            "arbitration.published",
            "arbitration.halted",
            "arbitration.resumed"
        ]
    );
    // 每份裁决都保存了签署内容与断言，challenge 可由签署内容重算
    let (intent, challenge): (String, Vec<u8>) = sqlx::query_as(
        "SELECT signed_intent, challenge FROM arbitration_rulings WHERE case_id = $1 ORDER BY id DESC LIMIT 1",
    )
    .bind(case)
    .fetch_one(&db)
    .await
    .unwrap();
    let digest = Sha256::digest([b"akari-ruling\0".as_slice(), intent.as_bytes()].concat());
    assert_eq!(digest.as_slice(), challenge.as_slice());

    // 执行前叫停后改判：重新公示 72 小时；申诉已用过，不能再申诉；期满才可执行
    let r = call(
        &app,
        "POST",
        &halt,
        Some(json!({"reason": "发现新证据"})),
        &owner_c,
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    let r = rule(
        &app,
        &mut ao,
        &owner_c,
        case,
        ruling("owner", 10_000, "新证据：卖家伪造物流"),
    )
    .await;
    assert_eq!(r.body["outcome"], "published", "{:?}", r.body);
    assert_eq!(r.body["status"], "appeal_window");
    assert_eq!(
        get_case(&app, &c1, case).await.body["execution"],
        Value::Null
    );
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/arbitration/cases/{case}/appeal"),
        Some(json!({"reason": "再申诉"})),
        &buyer_c,
    )
    .await;
    assert_eq!(r.body["code"], "CASE_STATE_INVALID");
    forum_db::arbitration::tick(
        &db,
        OffsetDateTime::now_utc() + Duration::hours(73),
        Duration::hours(48),
    )
    .await
    .unwrap();
    let d = get_case(&app, &c1, case).await.body;
    assert_eq!(d["status"], "ready");
    let data: Bytes = d["execution"]["data"].as_str().unwrap().parse().unwrap();
    assert_eq!(
        Escrow::resolveCall::abi_decode(&data).unwrap().toBuyerBps,
        U256::from(10_000)
    );
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn challenge_is_bound_to_the_exact_ruling(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let (buyer, _) = user(&app, &db).await;
    let (seller, _) = user(&app, &db).await;
    let (admin, c) = user(&app, &db).await;
    staff(&db, admin, ADMIN).await;
    let (_, case) = disputed_order(&db, buyer, seller, 50).await;
    tick(&db).await;
    assert_eq!(seated(&db, case, "panel").await, [admin]);
    let mut auth = enroll(&app, &db, admin, &c).await;
    let options = format!("/api/v1/arbitration/cases/{case}/ruling/options");

    // 举证期未结束不能裁决
    let r = call(
        &app,
        "POST",
        &options,
        Some(ruling("panel", 5_000, "各半")),
        &c,
    )
    .await;
    assert_eq!(r.body["code"], "EVIDENCE_WINDOW_OPEN");
    end_evidence(&db, case).await;
    // 接案后才与当事人交易：同样回避
    let late: i64 = sqlx::query_scalar(
        "INSERT INTO escrow_orders (chain_id, contract, chain_order_id, buyer, seller, buyer_id, seller_id, token,
           amount, instant, offer_hash, listing_hash, state, level, created_block, updated_block)
         VALUES (421614, decode(repeat('44', 20), 'hex'), 999, decode(repeat('b1', 20), 'hex'),
           decode(repeat('c1', 20), 'hex'), $1, $2, decode(repeat('33', 20), 'hex'), 5000000, false,
           decode(repeat('ab', 32), 'hex'), decode(repeat('ab', 32), 'hex'), 1, 'latest', 1, 1)
         RETURNING id",
    )
    .bind(admin)
    .bind(seller)
    .fetch_one(&db)
    .await
    .unwrap();
    let r = call(
        &app,
        "POST",
        &options,
        Some(ruling("panel", 5_000, "各半")),
        &c,
    )
    .await;
    assert_eq!(r.body["code"], "FORBIDDEN");
    sqlx::query("DELETE FROM escrow_orders WHERE id = $1")
        .bind(late)
        .execute(&db)
        .await
        .unwrap();
    // 订单没有保证金，不能赔付保证金；比例不能超过 100%
    let mut bad = ruling("panel", 5_000, "各半");
    bad["bond_to_buyer_bps"] = json!(1);
    assert_eq!(
        call(&app, "POST", &options, Some(bad), &c).await.body["code"],
        "INVALID_PARAMETER"
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &options,
            Some(ruling("panel", 10_001, "x")),
            &c
        )
        .await
        .body["code"],
        "INVALID_PARAMETER"
    );
    // 钱包页面来源不行
    let r = call_from(
        &app,
        WALLET_ORIGIN,
        "POST",
        &options,
        Some(ruling("panel", 5_000, "各半")),
        &c,
    )
    .await;
    assert_eq!(r.body["code"], "ORIGIN_REJECTED");

    // 确认的是 50%，提交时改成 60%：拒绝；同一次确认作废，原样再交也不行
    let signed = ruling("panel", 5_000, "各半");
    let (opts, cred) = sign(&app, &mut auth, &c, case, &signed).await;
    let intent: Value = serde_json::from_str(opts.body["intent"].as_str().unwrap()).unwrap();
    assert_eq!(
        (intent["case_id"].clone(), intent["to_buyer_bps"].clone()),
        (json!(case), json!(5000))
    );
    assert_eq!(intent["principal_to_buyer"], "25000000");
    let r = submit(&app, &c, case, &ruling("panel", 6_000, "各半"), &cred).await;
    assert_eq!(r.body["code"], "RULING_MISMATCH", "{:?}", r.body);
    let r = submit(&app, &c, case, &signed, &cred).await;
    assert_eq!(r.body["code"], "PASSKEY_FAILED");

    // 改理由同样拒绝
    let (_, cred) = sign(&app, &mut auth, &c, case, &signed).await;
    let r = submit(&app, &c, case, &ruling("panel", 5_000, "全给买家"), &cred).await;
    assert_eq!(r.body["code"], "RULING_MISMATCH");

    // 截获的旧断言不能用于后来开始的确认（challenge 含一次性随机数）
    let (_, old) = sign(&app, &mut auth, &c, case, &signed).await;
    let (_, _fresh) = sign(&app, &mut auth, &c, case, &signed).await;
    let r = submit(&app, &c, case, &signed, &old).await;
    assert_eq!(r.body["code"], "PASSKEY_FAILED");

    // 一致才通过；单人案件直接公示
    let r = rule(&app, &mut auth, &c, case, signed).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    assert_eq!(r.body["outcome"], "published");
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM arbitration_rulings WHERE case_id = $1")
        .bind(case)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(n, 1);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn broken_chat_chain_blocks_rulings(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let (buyer, buyer_c) = user(&app, &db).await;
    let (seller, _) = user(&app, &db).await;
    let (admin, c) = user(&app, &db).await;
    let (owner, _) = user(&app, &db).await;
    staff(&db, admin, ADMIN).await;
    staff(&db, owner, OWNER).await;
    let (order, case) = disputed_order(&db, buyer, seller, 50).await;
    sqlx::query("DELETE FROM staff_roles WHERE user_id = $1")
        .bind(owner)
        .execute(&db)
        .await
        .unwrap();
    tick(&db).await;
    staff(&db, owner, OWNER).await;
    end_evidence(&db, case).await;
    let mut auth = enroll(&app, &db, admin, &c).await;
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/orders/{order}/messages"),
        Some(json!({"body": "货呢？"})),
        &buyer_c,
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);

    // 确认之后、提交之前，服务器上的聊天记录被插入一条哈希对不上的消息
    let body = ruling("panel", 10_000, "卖家未发货");
    let (_, cred) = sign(&app, &mut auth, &c, case, &body).await;
    sqlx::query(
        "INSERT INTO order_messages (order_id, sender_id, kind, body, prev_hash, hash, created_at)
         SELECT $1, $2, 'text', '我同意退款', hash, decode(repeat('00', 32), 'hex'), now()
         FROM order_messages WHERE order_id = $1 ORDER BY id DESC LIMIT 1",
    )
    .bind(order)
    .bind(seller)
    .execute(&db)
    .await
    .unwrap();
    let r = submit(&app, &c, case, &body, &cred).await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    assert_eq!(r.body["code"], "CHAT_CHAIN_BROKEN");
    let d = get_case(&app, &c, case).await.body;
    assert_eq!(d["chat"]["verification"]["ok"], false);
    assert_eq!(d["rulings"], json!([]));
    // 记入审计日志并提醒站长；之后连确认都开始不了
    let alerts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox WHERE kind = 'notify.arbitration_chat_chain_broken'",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(alerts, 1);
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/arbitration/cases/{case}/ruling/options"),
        Some(body),
        &c,
    )
    .await;
    assert_eq!(r.body["code"], "CHAT_CHAIN_BROKEN");
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn statements_within_the_evidence_window(db: PgPool) {
    let Some(app) = build(db.clone()) else { return };
    let (buyer, buyer_c) = user(&app, &db).await;
    let (seller, seller_c) = user(&app, &db).await;
    let (admin, c) = user(&app, &db).await;
    let (_, outsider_c) = user(&app, &db).await;
    staff(&db, admin, ADMIN).await;
    let (_, case) = disputed_order(&db, buyer, seller, 50).await;
    tick(&db).await;
    let upload = |owner: i64| {
        let db = db.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "INSERT INTO uploads (user_id, status, content_type, size, tmp_key, width, height, variants)
                 VALUES ($1, 'done', 'image/png', 10, md5(random()::text), 10, 10,
                         '[{\"width\": 10, \"height\": 10, \"url\": \"https://img.test/a.webp\"}]')
                 RETURNING id",
            )
            .bind(owner)
            .fetch_one(&db)
            .await
            .unwrap()
        }
    };
    let mine = upload(buyer).await;
    let theirs = upload(seller).await;
    let path = format!("/api/v1/arbitration/cases/{case}/statements");
    let r = call(
        &app,
        "POST",
        &path,
        Some(json!({"body": "截图", "upload_ids": [theirs]})),
        &buyer_c,
    )
    .await;
    assert_eq!(r.body["code"], "INVALID_PARAMETER", "only own uploads");
    let r = call(
        &app,
        "POST",
        &path,
        Some(json!({"body": "截图如下", "upload_ids": [mine]})),
        &buyer_c,
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    assert_eq!(
        call(&app, "POST", &path, Some(json!({"body": "x"})), &outsider_c)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&app, "POST", &path, Some(json!({"body": "x"})), &c)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    // 对方与仲裁员都看得到陈述与图片
    for cookies in [&seller_c, &c] {
        let d = get_case(&app, cookies, case).await.body;
        assert_eq!(d["statements"][0]["body"], "截图如下");
        assert_eq!(
            d["statements"][0]["uploads"][0]["variants"][0]["url"],
            "https://img.test/a.webp"
        );
    }
    end_evidence(&db, case).await;
    let r = call(
        &app,
        "POST",
        &path,
        Some(json!({"body": "补充"})),
        &seller_c,
    )
    .await;
    assert_eq!(r.body["code"], "EVIDENCE_WINDOW_CLOSED");
    // 未公示的案件没有公示页
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/arbitration/public/{case}"),
        None,
        &[],
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}
