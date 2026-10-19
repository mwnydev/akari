//! 账号交易信誉（含防刷）、交易与发帖经验、等级、用户名。需要 PostgreSQL（`DATABASE_URL`）。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

use forum_db::exp::{self, Grant};
use forum_db::users::{self, SetUsername, TradeCounts};
use forum_db::{PgPool, reputation};

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

/// 一张订单。`delivered`：一次性订单有确认期限、分期订单已开始。
#[derive(Clone, Copy)]
struct Order {
    buyer: i64,
    seller: i64,
    usdt: i64,
    state: i16,
    finalized: bool,
    stream: bool,
    delivered: bool,
    ruling_bps: Option<i32>,
    /// 建单时间：几天前。
    days_ago: i32,
}

impl Order {
    fn released(buyer: i64, seller: i64, usdt: i64) -> Self {
        Self {
            buyer,
            seller,
            usdt,
            state: 3,
            finalized: true,
            stream: false,
            delivered: true,
            ruling_bps: None,
            days_ago: 1,
        }
    }
}

async fn order(db: &PgPool, o: Order) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO escrow_orders (chain_id, contract, chain_order_id, mode, buyer, seller, buyer_id, seller_id,
           token, amount, instant, offer_hash, listing_hash, state, confirm_deadline, started_at,
           ruling_to_buyer_bps, level, created_block, updated_block, created_at)
         VALUES (421614, decode(repeat('44', 20), 'hex'),
           (SELECT coalesce(max(chain_order_id), 0) + 1 FROM escrow_orders), $1,
           decode(repeat('b1', 20), 'hex'), decode(repeat('c1', 20), 'hex'), $2, $3,
           decode(repeat('33', 20), 'hex'), $4::bigint * 1000000, false, decode(repeat('ab', 32), 'hex'),
           decode(repeat('ab', 32), 'hex'), $5,
           CASE WHEN $6 AND $1 = 0 THEN now() END, CASE WHEN $6 AND $1 = 1 THEN now() END,
           $7, $8::text::chain_level, 1, 1, now() - make_interval(days => $9))
         RETURNING id",
    )
    .bind(i16::from(o.stream))
    .bind(o.buyer)
    .bind(o.seller)
    .bind(o.usdt)
    .bind(o.state)
    .bind(o.delivered)
    .bind(o.ruling_bps)
    .bind(if o.finalized { "finalized" } else { "safe" })
    .bind(o.days_ago)
    .fetch_one(db)
    .await
    .unwrap()
}

async fn stats(db: &PgPool, user_id: i64) -> (TradeCounts, String) {
    let p = users::profile(db, user_id).await.unwrap().unwrap();
    (p.trade, p.volume)
}

async fn exp_of(db: &PgPool, user_id: i64) -> (i16, i64) {
    let p = users::profile(db, user_id).await.unwrap().unwrap();
    (p.level, p.exp)
}

async fn apply(db: &PgPool, ids: &[i64]) -> Vec<i64> {
    let mut tx = db.begin().await.unwrap();
    let changed = reputation::apply_finalized(&mut tx, ids).await.unwrap();
    tx.commit().await.unwrap();
    changed
}

fn counts(success: i32, sf: i32, bf: i32, lost: i32, settled: i32) -> TradeCounts {
    TradeCounts {
        success,
        seller_fault_fail: sf,
        buyer_fault_fail: bf,
        disputes_lost: lost,
        settled,
    }
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn outcome_table_follows_plan(db: PgPool) {
    chain(&db).await;
    let [a, b, c, d, e, f, g, h] = [
        user(&db, "a").await,
        user(&db, "b").await,
        user(&db, "c").await,
        user(&db, "d").await,
        user(&db, "e").await,
        user(&db, "f").await,
        user(&db, "g").await,
        user(&db, "h").await,
    ];
    let mut ids = vec![
        // 确认放款：双方成功
        order(&db, Order::released(a, b, 50)).await,
        // 卖家未发货退款：卖家失败，买家不计
        order(
            &db,
            Order {
                state: 4,
                delivered: false,
                ..Order::released(c, d, 50)
            },
        )
        .await,
        // 仲裁全额判给买家：卖家失败 + 败诉
        order(
            &db,
            Order {
                state: 5,
                ruling_bps: Some(10_000),
                ..Order::released(e, f, 50)
            },
        )
        .await,
        // 仲裁判给卖家：卖家成功，买家失败 + 败诉
        order(
            &db,
            Order {
                state: 5,
                ruling_bps: Some(0),
                ..Order::released(g, h, 50)
            },
        )
        .await,
    ];
    // 发货后卖家主动退款、协商和解（无裁决）：都只计和解次数
    ids.push(
        order(
            &db,
            Order {
                state: 4,
                days_ago: 70,
                ..Order::released(a, b, 50)
            },
        )
        .await,
    );
    ids.push(
        order(
            &db,
            Order {
                state: 5,
                days_ago: 140,
                ..Order::released(a, b, 50)
            },
        )
        .await,
    );
    // 分期：未开始服务就退款 = 卖家失败
    ids.push(
        order(
            &db,
            Order {
                state: 4,
                stream: true,
                delivered: false,
                ..Order::released(e, d, 50)
            },
        )
        .await,
    );
    let changed = apply(&db, &ids).await;
    for u in [a, b, c, d, e, f, g, h] {
        assert!(changed.contains(&u), "user {u} stats changed");
    }

    assert_eq!(
        stats(&db, a).await,
        (counts(1, 0, 0, 0, 2), "50000000".into())
    );
    assert_eq!(
        stats(&db, b).await,
        (counts(1, 0, 0, 0, 2), "50000000".into())
    );
    assert_eq!(stats(&db, c).await.0, counts(0, 0, 0, 0, 0));
    assert_eq!(stats(&db, d).await.0, counts(0, 2, 0, 0, 0));
    assert_eq!(stats(&db, e).await.0, counts(0, 0, 0, 0, 0));
    assert_eq!(stats(&db, f).await.0, counts(0, 1, 0, 1, 0));
    assert_eq!(stats(&db, g).await.0, counts(0, 0, 1, 1, 0));
    assert_eq!(
        stats(&db, h).await,
        (counts(1, 0, 0, 0, 0), "50000000".into())
    );

    // 交易经验：成功订单双方 +10，败诉方 −100
    assert_eq!(exp_of(&db, a).await.1, 10);
    assert_eq!(exp_of(&db, b).await.1, 10);
    assert_eq!(exp_of(&db, f).await.1, -100);
    assert_eq!(exp_of(&db, g).await.1, -100);
    assert_eq!(exp_of(&db, h).await.1, 0, "won dispute: no trade exp");

    // 重复执行：不重复发经验，统计不变
    assert!(apply(&db, &ids).await.is_empty());
    assert_eq!(exp_of(&db, a).await.1, 10);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn anti_gaming_rules(db: PgPool) {
    chain(&db).await;
    let [a, b, c] = [
        user(&db, "a").await,
        user(&db, "b").await,
        user(&db, "c").await,
    ];
    let ids = vec![
        order(&db, Order::released(a, b, 50)).await,
        // 同一对用户 30 天窗口内的第二笔（反方向也算同一对）：不计
        order(&db, Order::released(b, a, 80)).await,
        // 不足 10 USDT：不计
        order(&db, Order::released(a, c, 9)).await,
        // 自己和自己：不计
        order(&db, Order::released(c, c, 50)).await,
        // 未最终确认：不计
        order(
            &db,
            Order {
                finalized: false,
                ..Order::released(b, c, 50)
            },
        )
        .await,
        // 10–20 USDT：计入信誉但没有交易经验
        order(&db, Order::released(c, b, 15)).await,
        // 同一对用户、但在 60 天前：另一个窗口，计入
        order(
            &db,
            Order {
                days_ago: 61,
                ..Order::released(a, b, 30)
            },
        )
        .await,
    ];
    apply(&db, &ids).await;
    assert_eq!(
        stats(&db, a).await,
        (counts(2, 0, 0, 0, 0), "80000000".into())
    );
    assert_eq!(
        stats(&db, b).await,
        (counts(3, 0, 0, 0, 0), "95000000".into())
    );
    assert_eq!(
        stats(&db, c).await,
        (counts(1, 0, 0, 0, 0), "15000000".into())
    );
    assert_eq!(exp_of(&db, a).await.1, 20);
    assert_eq!(exp_of(&db, b).await.1, 20);
    assert_eq!(exp_of(&db, c).await.1, 0);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn daily_recompute_repairs_drift(db: PgPool) {
    chain(&db).await;
    let [a, b] = [user(&db, "a").await, user(&db, "b").await];
    let id = order(&db, Order::released(a, b, 50)).await;
    apply(&db, &[id]).await;
    assert!(
        reputation::refresh_stats(&db, &[a, b])
            .await
            .unwrap()
            .is_empty(),
        "no drift"
    );
    sqlx::query("UPDATE user_trade_stats SET success = 99 WHERE user_id = $1")
        .bind(a)
        .execute(&db)
        .await
        .unwrap();
    // 没有订单的用户不写统计行
    let c = user(&db, "c").await;
    assert_eq!(
        reputation::refresh_stats(&db, &[a, b, c]).await.unwrap(),
        vec![a]
    );
    assert_eq!(stats(&db, a).await.0.success, 1);
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM user_trade_stats")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(rows, 2);
}

fn grant(user_id: i64, key: &str) -> Grant<'_> {
    Grant {
        user_id,
        ref_id: None,
        day: time::OffsetDateTime::now_utc().date(),
        dedupe: Some(key),
    }
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn exp_daily_cap_dedupe_and_levels(db: PgPool) {
    let u = user(&db, "u").await;
    let mut tx = db.begin().await.unwrap();
    for i in 0..7 {
        let key = format!("post:{i}");
        let r = exp::award(&mut tx, &exp::TOPIC, grant(u, &key))
            .await
            .unwrap();
        assert_eq!(r.is_some(), i < 5, "topic #{i}: 5 per day");
    }
    assert_eq!(
        exp::award(&mut tx, &exp::TOPIC, grant(u, "post:0"))
            .await
            .unwrap(),
        None
    );
    tx.commit().await.unwrap();
    assert_eq!(exp_of(&db, u).await, (1, 10));

    // 跨过 100 升到 2 级
    sqlx::query("UPDATE user_levels SET exp = 95 WHERE user_id = $1")
        .bind(u)
        .execute(&db)
        .await
        .unwrap();
    let mut tx = db.begin().await.unwrap();
    let g = Grant {
        dedupe: Some("pair:1:2:3"),
        ..grant(u, "")
    };
    assert_eq!(
        exp::award(&mut tx, &exp::TRADE, g).await.unwrap(),
        Some(true)
    );
    tx.commit().await.unwrap();
    assert_eq!(exp_of(&db, u).await, (2, 105));

    // 精华：设 +50、重复设不变、撤销扣回；等级不降
    let day = time::OffsetDateTime::now_utc().date();
    let mut tx = db.begin().await.unwrap();
    assert!(
        exp::set_featured(&mut tx, u, 9, true, day)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        exp::set_featured(&mut tx, u, 9, true, day)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        exp::set_featured(&mut tx, u, 9, false, day)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        exp::set_featured(&mut tx, u, 9, false, day)
            .await
            .unwrap()
            .is_none()
    );
    tx.commit().await.unwrap();
    assert_eq!(exp_of(&db, u).await, (2, 105));
    let mut tx = db.begin().await.unwrap();
    let g = Grant {
        dedupe: Some("order:1"),
        ..grant(u, "")
    };
    exp::award(&mut tx, &exp::DISPUTE_LOST, g).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(exp_of(&db, u).await, (2, 5), "level never drops");

    // 流水只追加
    assert!(
        sqlx::query("DELETE FROM exp_ledger")
            .execute(&db)
            .await
            .is_err()
    );
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn usernames_unique_case_insensitive_with_cooldown(db: PgPool) {
    let [a, b] = [user(&db, "a").await, user(&db, "b").await];
    assert_eq!(
        users::set_username(&db, a, "Alice").await.unwrap(),
        SetUsername::Done
    );
    assert_eq!(
        users::set_username(&db, b, "alice").await.unwrap(),
        SetUsername::Taken
    );
    assert_eq!(
        users::set_username(&db, a, "Alice2").await.unwrap(),
        SetUsername::TooSoon
    );
    sqlx::query("UPDATE users SET username_changed_at = now() - interval '31 days' WHERE id = $1")
        .bind(a)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(
        users::set_username(&db, a, "Alice2").await.unwrap(),
        SetUsername::Done
    );
    assert_eq!(
        users::set_username(&db, b, "ALICE").await.unwrap(),
        SetUsername::Done
    );
    assert_eq!(users::id_by_name(&db, "alice2").await.unwrap(), Some(a));

    let names: Vec<String> = users::search_prefix(&db, "ALI", 10)
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.username)
        .collect();
    assert_eq!(names, vec!["ALICE".to_owned(), "Alice2".to_owned()]);
    assert!(
        users::search_prefix(&db, "bob", 10)
            .await
            .unwrap()
            .is_empty()
    );
}
