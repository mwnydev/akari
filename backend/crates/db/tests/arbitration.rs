//! 仲裁案件：随订单链上状态开关（含区块回滚）、随机分配与自动回避、超时改派、两人裁决与分歧、申诉期、
//! 工作量统计、只追加。需要 PostgreSQL（`DATABASE_URL`）。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

use forum_core::arbitration::{RulingKind, TWO_PERSON_MIN_USDT};
use forum_db::arbitration::{self as arb, NewRuling, Outcome, Synced};
use forum_db::{PgPool, users};
use serde_json::json;
use time::{Duration, OffsetDateTime};

const DEADLINE: Duration = Duration::hours(48);

async fn chain(db: &PgPool) {
    sqlx::query(
        "INSERT INTO chains (chain_id, name, escrow_address, config_address, token_address, token_symbol,
           token_decimals, start_block, safe_depth, finalized_depth)
         VALUES (421614, 'test', decode(repeat('44', 20), 'hex'), decode(repeat('c0', 20), 'hex'),
           decode(repeat('33', 20), 'hex'), 'USDT', 6, 0, 240, 4800)",
    )
    .execute(db)
    .await
    .unwrap();
}

async fn user(db: &PgPool, name: &str) -> i64 {
    users::upsert_by_email(db, &format!("{name}@example.com"))
        .await
        .unwrap()
        .0
        .id
}

async fn role(db: &PgPool, user_id: i64, role: i16) {
    sqlx::query("INSERT INTO staff_roles (user_id, role) VALUES ($1, $2)")
        .bind(user_id)
        .bind(role)
        .execute(db)
        .await
        .unwrap();
}

/// 一张订单（`usdt` 整 USDT），返回订单 ID。
async fn order(db: &PgPool, buyer: i64, seller: i64, usdt: i64, state: i16) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO escrow_orders (chain_id, contract, chain_order_id, mode, buyer, seller, buyer_id, seller_id,
           token, amount, instant, offer_hash, listing_hash, state, disputed_at, level, created_block, updated_block)
         VALUES (421614, decode(repeat('44', 20), 'hex'),
           (SELECT coalesce(max(chain_order_id), 0) + 1 FROM escrow_orders), 0,
           decode(repeat('b1', 20), 'hex'), decode(repeat('c1', 20), 'hex'), $1, $2,
           decode(repeat('33', 20), 'hex'), $3::bigint * 1000000, false, decode(repeat('ab', 32), 'hex'),
           decode(repeat('ab', 32), 'hex'), $4, CASE WHEN $4 = 2 THEN now() END, 'latest', 1, 1)
         RETURNING id",
    )
    .bind(buyer)
    .bind(seller)
    .bind(usdt)
    .bind(state)
    .fetch_one(db)
    .await
    .unwrap()
}

/// 改订单链上状态（模拟 indexer 重新折叠后的结果）并同步案件。
async fn set_state(db: &PgPool, order_id: i64, state: i16, ruling: Option<i32>) -> Option<Synced> {
    sqlx::query(
        "UPDATE escrow_orders SET state = $2, ruling_to_buyer_bps = $3, ruling_bond_to_buyer_bps = $4 WHERE id = $1",
    )
    .bind(order_id)
    .bind(state)
    .bind(ruling)
    .bind(ruling.map(|_| 0))
    .execute(db)
    .await
    .unwrap();
    sync(db, order_id).await
}

async fn sync(db: &PgPool, order_id: i64) -> Option<Synced> {
    let mut tx = db.begin().await.unwrap();
    let s = arb::sync_order(&mut tx, order_id).await.unwrap();
    tx.commit().await.unwrap();
    s
}

async fn case(db: &PgPool, id: i64) -> arb::Case {
    arb::case_by_id(db, id).await.unwrap().unwrap()
}

/// 当前在岗或已提交的人（被改派走的不算）。
async fn seated(db: &PgPool, case_id: i64, role: &str) -> Vec<i64> {
    let mut v: Vec<i64> = arb::assignments(db, case_id)
        .await
        .unwrap()
        .into_iter()
        .filter(|a| a.role == role && a.counts())
        .map(|a| a.arbitrator_id)
        .collect();
    v.sort_unstable();
    v
}

async fn count(db: &PgPool, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(db).await.unwrap()
}

/// 结束举证期，让 worker 的期限从现在算起。
async fn end_evidence(db: &PgPool, case_id: i64) {
    sqlx::query(
        "UPDATE arbitration_cases SET evidence_until = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(case_id)
    .execute(db)
    .await
    .unwrap();
}

async fn passkey(db: &PgPool, user_id: i64) -> i64 {
    arb::insert_ruling_passkey(db, user_id, &user_id.to_be_bytes(), &json!({}))
        .await
        .unwrap()
        .unwrap()
}

/// 直接写一份裁决（Passkey 与哈希链在接口层校验，见 forum-api 的 arbitration_flow）。
async fn rule(db: &PgPool, case_id: i64, by: i64, kind: RulingKind, bps: i32) -> (i64, Outcome) {
    let pk = passkey(db, by).await;
    let mut tx = db.begin().await.unwrap();
    assert!(arb::lock_case(&mut tx, case_id).await.unwrap());
    let c = arb::case_by_id(&mut *tx, case_id).await.unwrap().unwrap();
    let assignment = arb::assignments(&mut *tx, case_id)
        .await
        .unwrap()
        .into_iter()
        .find(|a| a.arbitrator_id == by && a.is_active())
        .map(|a| a.id);
    let out = arb::submit_ruling(
        &mut tx,
        &c,
        &NewRuling {
            case_id,
            arbitrator_id: by,
            assignment_id: assignment,
            kind,
            to_buyer_bps: bps,
            bond_to_buyer_bps: 0,
            principal_to_buyer: "0".into(),
            principal_to_seller: "0".into(),
            bond_to_buyer: "0".into(),
            reason: "理由",
            chat_head: &[0; 32],
            chat_count: 0,
            signed_intent: "{}",
            challenge: &[1; 32],
            passkey_id: pk,
            assertion: &json!({}),
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    out
}

struct World {
    buyer: i64,
    seller: i64,
    owner: i64,
    admins: Vec<i64>,
}

async fn world(db: &PgPool, admins: usize) -> World {
    chain(db).await;
    let w = World {
        buyer: user(db, "buyer").await,
        seller: user(db, "seller").await,
        owner: user(db, "owner").await,
        admins: {
            let mut v = Vec::new();
            for i in 0..admins {
                v.push(user(db, &format!("admin{i}")).await);
            }
            v
        },
    };
    role(db, w.owner, 1).await;
    for a in &w.admins {
        role(db, *a, 3).await;
    }
    w
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn case_follows_dispute_reorg_and_execution(db: PgPool) {
    let w = world(&db, 1).await;
    let o = order(&db, w.buyer, w.seller, 50, 1).await;
    // 未争议的订单没有案件
    assert_eq!(sync(&db, o).await, None);

    let Some(Synced::Opened(id)) = set_state(&db, o, 2, None).await else {
        panic!("case not opened")
    };
    let c = case(&db, id).await;
    assert_eq!((c.status.as_str(), c.panel_size), ("reviewing", 1));
    assert!(c.evidence_until > OffsetDateTime::now_utc() + Duration::hours(71));
    // 重复同步不变（幂等）
    assert_eq!(sync(&db, o).await, None);
    arb::tick(&db, OffsetDateTime::now_utc(), DEADLINE)
        .await
        .unwrap();
    let panel = seated(&db, id, "panel").await;
    assert_eq!(panel.len(), 1);

    // 争议交易被区块回滚：订单回到已付款 → 案件关闭，分配保留
    assert_eq!(
        set_state(&db, o, 1, None).await,
        Some(Synced::Closed(id, "reverted"))
    );
    assert_eq!(
        case(&db, id).await.close_reason.as_deref(),
        Some("reverted")
    );
    // 关闭的案件不再分配
    assert_eq!(
        arb::tick(
            &db,
            OffsetDateTime::now_utc() + Duration::days(10),
            DEADLINE
        )
        .await
        .unwrap()
        .total(),
        0
    );
    // 争议在主链上重新打包：同一案件重新打开，原分配还在
    assert_eq!(set_state(&db, o, 2, None).await, Some(Synced::Reopened(id)));
    let c = case(&db, id).await;
    assert!(c.is_open() && c.status == "reviewing");
    assert_eq!(seated(&db, id, "panel").await, panel);

    // 还没公示就被执行：关闭为 executed，并提醒站长
    assert_eq!(
        set_state(&db, o, 5, Some(10_000)).await,
        Some(Synced::Closed(id, "executed"))
    );
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM outbox WHERE kind = 'notify.arbitration_execution_mismatch'"
        )
        .await,
        1
    );
    let actions: Vec<String> =
        sqlx::query_scalar("SELECT action FROM admin_audit_logs WHERE target = $1 ORDER BY id")
            .bind(format!("arbitration_case:{id}"))
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(
        actions,
        [
            "arbitration.opened",
            "arbitration.assigned",
            "arbitration.closed",
            "arbitration.reopened",
            "arbitration.closed"
        ]
    );
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn assignment_recuses_related_staff(db: PgPool) {
    let w = world(&db, 4).await;
    let [traded, party_admin, free1, free2] = w.admins[..] else {
        unreachable!()
    };
    // admin0 30 天前与买家有过订单；admin1 自己就是卖家
    let old = order(&db, traded, w.buyer, 20, 3).await;
    sqlx::query("UPDATE escrow_orders SET created_at = now() - interval '30 days' WHERE id = $1")
        .bind(old)
        .execute(&db)
        .await
        .unwrap();
    // free2 一年前与卖家交易过：超过 90 天，不回避
    let ancient = order(&db, w.seller, free2, 20, 3).await;
    sqlx::query("UPDATE escrow_orders SET created_at = now() - interval '200 days' WHERE id = $1")
        .bind(ancient)
        .execute(&db)
        .await
        .unwrap();
    let small = order(&db, w.buyer, party_admin, 50, 2).await;
    let Some(Synced::Opened(id)) = sync(&db, small).await else {
        panic!()
    };
    assert!(arb::is_recused(&db, id, traded).await.unwrap());
    assert!(arb::is_recused(&db, id, party_admin).await.unwrap());
    assert!(arb::is_recused(&db, id, w.buyer).await.unwrap());
    assert!(!arb::is_recused(&db, id, free1).await.unwrap());
    assert!(!arb::is_recused(&db, id, free2).await.unwrap());
    assert!(arb::is_eligible(&db, w.owner).await.unwrap());
    assert!(!arb::is_eligible(&db, w.buyer).await.unwrap());

    // 小额：一人；只可能是没回避的人（含站长）
    arb::tick(&db, OffsetDateTime::now_utc(), DEADLINE)
        .await
        .unwrap();
    let picked = seated(&db, id, "panel").await;
    assert_eq!(picked.len(), 1);
    assert!([free1, free2, w.owner].contains(&picked[0]), "{picked:?}");

    // 大额（≥ 200 USDT）：两人，不抽站长（站长负责分歧终裁）
    let big = order(&db, w.buyer, party_admin, TWO_PERSON_MIN_USDT, 2).await;
    let Some(Synced::Opened(big_id)) = sync(&db, big).await else {
        panic!()
    };
    assert_eq!(case(&db, big_id).await.panel_size, 2);
    arb::tick(&db, OffsetDateTime::now_utc(), DEADLINE)
        .await
        .unwrap();
    assert_eq!(seated(&db, big_id, "panel").await, {
        let mut v = vec![free1, free2];
        v.sort_unstable();
        v
    });
    // 补位找不到人时提醒站长一次：free2 是买家、admin0 刚与卖家交易过，只剩 free1
    order(&db, traded, party_admin, 20, 3).await;
    let big2 = order(&db, w.buyer, party_admin, 300, 2).await;
    let Some(Synced::Opened(_)) = sync(&db, big2).await else {
        panic!()
    };
    sqlx::query("UPDATE escrow_orders SET buyer_id = $1 WHERE id = $2")
        .bind(free2)
        .bind(big2)
        .execute(&db)
        .await
        .unwrap();
    let t = arb::tick(&db, OffsetDateTime::now_utc(), DEADLINE)
        .await
        .unwrap();
    assert_eq!((t.assigned, t.stalled), (1, 1));
    let t = arb::tick(&db, OffsetDateTime::now_utc(), DEADLINE)
        .await
        .unwrap();
    assert_eq!(t.stalled, 0, "alerted only once");
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM outbox WHERE kind = 'notify.arbitration_stalled'"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM outbox WHERE kind = 'email.arbitration_alert'"
        )
        .await,
        1
    );
    // 管理员群 Telegram 告警也只有一条，且只带案件号与标题
    let alerts: Vec<serde_json::Value> =
        sqlx::query_scalar("SELECT payload FROM outbox WHERE kind = 'staff_alert'")
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    assert_eq!(alerts[0]["event"], "arbitration.stalled");
    let text = alerts[0]["text"].as_str().unwrap();
    assert!(
        text.starts_with("仲裁案件 #") && text.contains("找不到可分配的仲裁员"),
        "{text}"
    );
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn overdue_arbitrator_is_reassigned(db: PgPool) {
    let w = world(&db, 2).await;
    let o = order(&db, w.buyer, w.seller, 50, 2).await;
    let Some(Synced::Opened(id)) = sync(&db, o).await else {
        panic!()
    };
    // 站长不当这个案件的仲裁员，结果可预期
    sqlx::query("DELETE FROM staff_roles WHERE user_id = $1")
        .bind(w.owner)
        .execute(&db)
        .await
        .unwrap();
    end_evidence(&db, id).await;
    let now = OffsetDateTime::now_utc();
    arb::tick(&db, now, DEADLINE).await.unwrap();
    let first = arb::assignments(&db, id).await.unwrap().remove(0);
    assert!(
        first.due_at > now + Duration::hours(47)
            && first.due_at <= now + DEADLINE + Duration::seconds(1)
    );
    // 期限内不改派
    let t = arb::tick(&db, now + Duration::hours(47), DEADLINE)
        .await
        .unwrap();
    assert_eq!(t.reassigned, 0);
    let later = now + Duration::hours(49);
    let t = arb::tick(&db, later, DEADLINE).await.unwrap();
    assert_eq!(t.reassigned, 1);
    let all = arb::assignments(&db, id).await.unwrap();
    assert_eq!(all[0].ended_reason.as_deref(), Some("reassigned"));
    assert_ne!(all[1].arbitrator_id, first.arbitrator_id);
    assert!(all[1].is_active());
    // 被改派走的人收到通知，也不再能看这个订单
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM outbox WHERE kind = 'notify.arbitration_reassigned'"
        )
        .await,
        1
    );
    assert!(
        !arb::can_review_order(&db, o, first.arbitrator_id)
            .await
            .unwrap()
    );
    assert!(
        arb::can_review_order(&db, o, all[1].arbitrator_id)
            .await
            .unwrap()
    );
    // 再超时但已没有别人：留在岗位上，提醒站长
    let t = arb::tick(&db, later + Duration::hours(49), DEADLINE)
        .await
        .unwrap();
    assert_eq!((t.reassigned, t.stalled), (0, 1));

    rule(&db, id, all[1].arbitrator_id, RulingKind::Panel, 10_000).await;
    let stats = arb::workload(&db, now - Duration::days(1)).await.unwrap();
    let of = |u: i64| stats.iter().find(|s| s.arbitrator_id == u).unwrap();
    let late = of(first.arbitrator_id);
    assert_eq!((late.assigned, late.ruled, late.reassigned), (1, 0, 1));
    let done = of(all[1].arbitrator_id);
    assert_eq!(
        (done.assigned, done.ruled, done.on_time, done.open),
        (1, 1, 1, 0)
    );
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn two_person_disagreement_goes_to_owner_then_appeal_window(db: PgPool) {
    let w = world(&db, 3).await;
    let o = order(&db, w.buyer, w.seller, 200, 2).await;
    let Some(Synced::Opened(id)) = sync(&db, o).await else {
        panic!()
    };
    end_evidence(&db, id).await;
    arb::tick(&db, OffsetDateTime::now_utc(), DEADLINE)
        .await
        .unwrap();
    let panel = seated(&db, id, "panel").await;
    assert_eq!(panel.len(), 2);
    assert!(!panel.contains(&w.owner));

    assert_eq!(
        rule(&db, id, panel[0], RulingKind::Panel, 10_000).await.1,
        Outcome::Waiting
    );
    assert_eq!(case(&db, id).await.status, "reviewing");
    assert_eq!(
        rule(&db, id, panel[1], RulingKind::Panel, 5_000).await.1,
        Outcome::Disagreed
    );
    assert_eq!(case(&db, id).await.status, "owner_review");
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM outbox WHERE kind = 'notify.arbitration_owner_review'"
        )
        .await,
        1
    );

    let (owner_ruling, out) = rule(&db, id, w.owner, RulingKind::Owner, 5_000).await;
    assert_eq!(out, Outcome::Published);
    let c = case(&db, id).await;
    assert_eq!(c.status, "appeal_window");
    assert_eq!(c.final_ruling_id, Some(owner_ruling));
    let until = c.appeal_until.unwrap();
    let published = c.published_at.unwrap();
    assert_eq!(until - published, Duration::hours(72));
    // 双方收到公示通知，站长收到裁决推送（站内 + 邮件，含理由）
    let notified: Vec<i64> = sqlx::query_scalar(
        "SELECT (payload->>'to')::bigint FROM outbox WHERE kind = 'notify.arbitration_published' ORDER BY 1",
    )
    .fetch_all(&db)
    .await
    .unwrap();
    let mut expect = vec![w.buyer, w.seller, w.owner];
    expect.sort_unstable();
    assert_eq!(notified, expect);
    let mail: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM outbox WHERE kind = 'email.arbitration_alert' AND payload->>'event' = 'published'",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(mail["detail"]["reason"], "理由");
    assert_eq!(mail["detail"]["to_buyer_bps"], 5000);

    // 申诉期内不变，过了才可执行
    let t = arb::tick(&db, until - Duration::minutes(1), DEADLINE)
        .await
        .unwrap();
    assert_eq!(t.ready, 0);
    assert_eq!(case(&db, id).await.status, "appeal_window");
    let t = arb::tick(&db, until + Duration::seconds(1), DEADLINE)
        .await
        .unwrap();
    assert_eq!(t.ready, 1);
    let c = case(&db, id).await;
    assert_eq!(c.status, "ready");
    assert!(c.ready_at.is_some());

    // 已公示裁决被原样执行：关闭，不提醒
    assert_eq!(
        set_state(&db, o, 5, Some(5_000)).await,
        Some(Synced::Closed(id, "executed"))
    );
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM outbox WHERE kind = 'notify.arbitration_execution_mismatch'"
        )
        .await,
        0
    );
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn appeal_is_reviewed_by_someone_who_took_no_part(db: PgPool) {
    let w = world(&db, 2).await;
    let o = order(&db, w.buyer, w.seller, 50, 2).await;
    let Some(Synced::Opened(id)) = sync(&db, o).await else {
        panic!()
    };
    sqlx::query("DELETE FROM staff_roles WHERE user_id = $1")
        .bind(w.owner)
        .execute(&db)
        .await
        .unwrap();
    end_evidence(&db, id).await;
    arb::tick(&db, OffsetDateTime::now_utc(), DEADLINE)
        .await
        .unwrap();
    let panel = seated(&db, id, "panel").await;
    rule(&db, id, panel[0], RulingKind::Panel, 0).await;
    let c = case(&db, id).await;
    let mut tx = db.begin().await.unwrap();
    arb::appeal(&mut tx, &c, w.buyer, "不服").await.unwrap();
    tx.commit().await.unwrap();
    arb::tick(&db, OffsetDateTime::now_utc(), DEADLINE)
        .await
        .unwrap();
    let reviewer = seated(&db, id, "appeal").await;
    assert_eq!(reviewer.len(), 1);
    assert_ne!(reviewer[0], panel[0]);
    let (_, out) = rule(&db, id, reviewer[0], RulingKind::Appeal, 10_000).await;
    assert_eq!(out, Outcome::Final);
    let c = case(&db, id).await;
    assert_eq!(c.status, "ready");
    assert_eq!(c.appeal_by, Some(w.buyer));
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn rulings_and_statements_are_append_only(db: PgPool) {
    let w = world(&db, 1).await;
    let o = order(&db, w.buyer, w.seller, 50, 2).await;
    let Some(Synced::Opened(id)) = sync(&db, o).await else {
        panic!()
    };
    let mut tx = db.begin().await.unwrap();
    arb::add_statement(&mut tx, id, w.buyer, "没收到货", &[])
        .await
        .unwrap();
    tx.commit().await.unwrap();
    rule(&db, id, w.owner, RulingKind::Owner, 10_000).await;
    for sql in [
        "UPDATE arbitration_rulings SET to_buyer_bps = 0",
        "DELETE FROM arbitration_rulings",
        "UPDATE arbitration_statements SET body = 'x'",
        "DELETE FROM arbitration_statements",
        "TRUNCATE arbitration_rulings CASCADE",
    ] {
        let err = sqlx::query(sql).execute(&db).await.unwrap_err();
        assert!(err.to_string().contains("append-only"), "{sql}: {err}");
    }
}
