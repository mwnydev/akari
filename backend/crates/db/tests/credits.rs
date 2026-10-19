//! 免费代付提币次数：只增流水、开通奖励一次、订单终态最终确认后双方各一次（重放、回滚重放不重复）、
//! 签名过期且没上链才退回（只退一次；代发前作废的也要等过期）、上链失败不退、当天次数与同 nonce 旧签名。
//! 需要 PostgreSQL。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

use forum_db::PgPool;
use forum_db::chain::{self, ChainRow, OrderUpsert};
use forum_db::credits::{self, Nonce, Usage};
use forum_db::sponsor::{self as sp, NewSponsorship, Outcome, SponsorshipStatus};
use time::{Duration, OffsetDateTime};

const CHAIN: i64 = 31_337;
const ESCROW: [u8; 20] = [0xe5; 20];
const SENDER: [u8; 20] = [5; 20];

/// 带本链钱包的用户。
async fn user(pool: &PgPool, n: u8) -> i64 {
    sqlx::query_scalar(
        "WITH u AS (INSERT INTO users (email) VALUES ('c' || $1 || '@example.com') RETURNING id),
              k AS (INSERT INTO passkeys (user_id, purpose, credential_id, credential)
                    SELECT id, 2, $2, '{}' FROM u RETURNING id, user_id)
         INSERT INTO wallets (user_id, chain_id, address, passkey_id) SELECT user_id, $3, $2, id FROM k
         RETURNING user_id",
    )
    .bind(i32::from(n))
    .bind(vec![n; 20])
    .bind(CHAIN)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn usage(pool: &PgPool, user_id: i64) -> Usage {
    let since = OffsetDateTime::now_utc() - Duration::days(1);
    credits::usage(pool, user_id, since, None).await.unwrap()
}

async fn ledger(pool: &PgPool, user_id: i64) -> Vec<(String, i16)> {
    sqlx::query_as(
        "SELECT reason::text, delta FROM withdraw_credit_ledger WHERE user_id = $1 ORDER BY id",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .unwrap()
}

/// 一笔代付提币签名（同 forum-sponsor：签名记录 + 扣 1 次）。
async fn sign_withdraw(pool: &PgPool, user_id: i64, nonce: &str, hash: u8) -> i64 {
    let mut tx = pool.begin().await.unwrap();
    sp::replace_signed(&mut *tx, CHAIN, &SENDER, nonce, &[hash; 32])
        .await
        .unwrap();
    let id = sp::insert(
        &mut *tx,
        NewSponsorship {
            user_id,
            chain_id: CHAIN,
            sender: &SENDER,
            nonce,
            user_op_hash: &[hash; 32],
            fee: "50000",
            valid_until: OffsetDateTime::now_utc() + Duration::minutes(2),
            signed_block: 1,
        },
    )
    .await
    .unwrap()
    .unwrap();
    credits::debit_withdraw(&mut *tx, user_id, id)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    id
}

fn outcome(hash: &[u8; 32], success: bool) -> Outcome<'_> {
    Outcome {
        user_op_hash: hash,
        success,
        tx_hash: &[0xab; 32],
        block_number: 9,
        actual_gas_cost: "1000",
    }
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn ledger_is_append_only_and_drives_the_balance(pool: PgPool) {
    let u = user(&pool, 1).await;
    assert_eq!(usage(&pool, u).await, Usage::default());
    assert!(credits::award_starter(&pool, u).await.unwrap());
    assert!(
        !credits::award_starter(&pool, u).await.unwrap(),
        "starter credit only once"
    );
    assert_eq!(usage(&pool, u).await.balance, 1);
    assert_eq!(ledger(&pool, u).await, [("starter".to_owned(), 1)]);

    for sql in [
        "UPDATE withdraw_credit_ledger SET delta = 5",
        "DELETE FROM withdraw_credit_ledger",
        "TRUNCATE withdraw_credit_ledger",
    ] {
        let e = sqlx::query(sql).execute(&pool).await.unwrap_err();
        assert!(e.to_string().contains("append-only"), "{sql}: {e}");
    }
    // 理由与增减、关联必须匹配。
    let bad = sqlx::query(
        "INSERT INTO withdraw_credit_ledger (user_id, delta, reason) VALUES ($1, 1, 'withdraw')",
    )
    .bind(u)
    .execute(&pool)
    .await;
    assert!(bad.is_err());
    assert_eq!(usage(&pool, u).await.balance, 1);
}

async fn setup_chain(pool: &PgPool) {
    chain::upsert_chain(
        pool,
        ChainRow {
            chain_id: CHAIN,
            name: "test",
            escrow: &ESCROW,
            stream: &[0x5e; 20],
            config: &[0xc0; 20],
            token: &[0xd1; 20],
            token_symbol: "mUSDT",
            token_decimals: 6,
            start_block: 1,
            safe_depth: 2,
            finalized_depth: 4,
        },
    )
    .await
    .unwrap();
}

/// 写入（或改写）订单 1，返回行 id。
async fn order(pool: &PgPool, buyer: u8, seller: u8, state: i16, level: &str) -> i64 {
    chain::upsert_order(
        pool,
        &OrderUpsert {
            chain_id: CHAIN,
            contract: &ESCROW,
            chain_order_id: 1,
            buyer: vec![buyer; 20],
            seller: vec![seller; 20],
            token: vec![0xd1; 20],
            amount: "5000000".into(),
            offer_hash: vec![1; 32],
            listing_hash: vec![2; 32],
            state,
            level,
            created_block: 1,
            updated_block: 2,
            ..OrderUpsert::default()
        },
    )
    .await
    .unwrap()
    .unwrap()
    .id
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn finalized_terminal_orders_award_both_parties_once(pool: PgPool) {
    setup_chain(&pool).await;
    let (buyer, seller) = (user(&pool, 1).await, user(&pool, 2).await);

    // 未到终态或未最终确认：不发。
    for (state, level) in [
        (1, "finalized"),
        (2, "finalized"),
        (3, "latest"),
        (3, "safe"),
    ] {
        let id = order(&pool, 1, 2, state, level).await;
        assert_eq!(credits::award_order(&pool, id).await.unwrap(), 0);
    }
    for state in [3, 4, 5] {
        let id = order(&pool, 1, 2, state, "finalized").await;
        let awarded = credits::award_order(&pool, id).await.unwrap();
        assert_eq!(awarded, if state == 3 { 2 } else { 0 }, "state {state}");
    }
    // 重放（同一事件再折叠一次）、回滚后订单清零再重放到最终确认：都不重复发。
    let id = order(&pool, 1, 2, 1, "latest").await;
    assert!(
        chain::clear_order(&pool, CHAIN, &ESCROW, 1)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(order(&pool, 1, 2, 4, "finalized").await, id);
    assert_eq!(credits::award_order(&pool, id).await.unwrap(), 0);
    assert_eq!(credits::award_order(&pool, id).await.unwrap(), 0);
    for u in [buyer, seller] {
        assert_eq!(ledger(&pool, u).await, [("order".to_owned(), 1)]);
        assert_eq!(usage(&pool, u).await.balance, 1);
    }

    // 自己和自己交易（同一用户两边）只算一次；不是本站用户的一方不发。
    let solo = user(&pool, 3).await;
    let other = chain::upsert_order(
        &pool,
        &OrderUpsert {
            chain_id: CHAIN,
            contract: &ESCROW,
            chain_order_id: 2,
            buyer: vec![3; 20],
            seller: vec![3; 20],
            token: vec![0xd1; 20],
            amount: "5000000".into(),
            offer_hash: vec![1; 32],
            listing_hash: vec![2; 32],
            state: 5,
            level: "finalized",
            created_block: 1,
            updated_block: 2,
            ..OrderUpsert::default()
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(credits::award_order(&pool, other.id).await.unwrap(), 1);
    assert_eq!(usage(&pool, solo).await.balance, 1);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn unused_signatures_refund_and_landed_ones_do_not(pool: PgPool) {
    let u = user(&pool, 1).await;
    credits::award_starter(&pool, u).await.unwrap();
    let since = OffsetDateTime::now_utc() - Duration::days(1);
    let nonce = |n| Nonce {
        chain_id: CHAIN,
        sender: &SENDER,
        nonce: n,
    };

    // 签名：扣 1 次，当天用掉 1 次。
    sign_withdraw(&pool, u, "1", 1).await;
    assert_eq!(
        usage(&pool, u).await,
        Usage {
            balance: 0,
            today: 1,
            held: 0
        }
    );
    // 同一 nonce 重签：旧签名扣的次数可以抵用，也不算当天多用一次。
    let u1 = credits::usage(&pool, u, since, Some(nonce("1")))
        .await
        .unwrap();
    assert_eq!((u1.balance + u1.held, u1.today), (1, 0));
    sign_withdraw(&pool, u, "1", 2).await;
    let u2 = credits::usage(&pool, u, since, Some(nonce("1")))
        .await
        .unwrap();
    // 余额暂为负；再重签一次，新旧两笔签名扣的都可抵用，可用的仍是 1 次。
    assert_eq!((u2.balance, u2.held, u2.today), (-1, 2, 0));
    assert_eq!(
        usage(&pool, u).await.today,
        1,
        "the replaced one not counted"
    );

    // 被取代的旧签名过期（对账没找到）：退回；重复关闭不重复退。
    for _ in 0..2 {
        sp::close(&pool, &[1; 32], SponsorshipStatus::Expired)
            .await
            .unwrap();
    }
    assert_eq!(usage(&pool, u).await.balance, 0);
    // 新签名上链但 callData 回滚：代付费照收，次数不退；之后对账再关闭也不动它。
    sp::settle(&pool, outcome(&[2; 32], false)).await.unwrap();
    sp::close(&pool, &[2; 32], SponsorshipStatus::Expired)
        .await
        .unwrap();
    assert_eq!(
        usage(&pool, u).await,
        Usage {
            balance: 0,
            today: 1,
            held: 0
        }
    );

    // 同一笔签名不能退两次（唯一键兜底）。
    sqlx::query(
        "INSERT INTO withdraw_credit_ledger (user_id, delta, reason, sponsorship_id)
         SELECT $1, 1, 'refund_expired', id FROM sponsorships WHERE user_op_hash = $2",
    )
    .bind(u)
    .bind([1u8; 32].as_slice())
    .execute(&pool)
    .await
    .expect_err("one refund per sponsorship");
    assert_eq!(
        ledger(&pool, u).await,
        [
            ("starter".to_owned(), 1),
            ("withdraw".to_owned(), -1),
            ("withdraw".to_owned(), -1),
            ("refund_expired".to_owned(), 1),
        ]
    );
}

/// 让签名过期并按对账任务的做法结束它（没找到链上结果）。
async fn expire_unlanded(pool: &PgPool, hash: u8) {
    sqlx::query(
        "UPDATE sponsorships SET valid_until = now() - interval '1 minute' WHERE user_op_hash = $1",
    )
    .bind([hash; 32].as_slice())
    .execute(pool)
    .await
    .unwrap();
    let due = sp::due_for_reconcile(pool, 0.0, 10).await.unwrap();
    assert!(
        due.iter().any(|d| d.user_op_hash == [hash; 32]),
        "expired signature is reconciled"
    );
    sp::close(pool, &[hash; 32], SponsorshipStatus::Expired)
        .await
        .unwrap();
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn dropped_signature_keeps_its_credit_until_it_expires(pool: PgPool) {
    let u = user(&pool, 1).await;
    credits::award_starter(&pool, u).await.unwrap();
    let since = OffsetDateTime::now_utc() - Duration::days(1);
    let nonce = |n| Nonce {
        chain_id: CHAIN,
        sender: &SENDER,
        nonce: n,
    };
    sign_withdraw(&pool, u, "3", 3).await;
    // 代发器作废：validUntil 前仍可能经第三方 bundler 上链，次数不退、当天次数照占。
    for _ in 0..2 {
        sp::close(&pool, &[3; 32], SponsorshipStatus::Dropped)
            .await
            .unwrap();
    }
    let dropped = Usage {
        balance: 0,
        today: 1,
        held: 0,
    };
    assert_eq!(usage(&pool, u).await, dropped);
    // 换个 nonce（并行的另一笔）没有次数可用；同一 nonce 重签可以抵用它扣的那次（两笔最多一笔上链）。
    let other = credits::usage(&pool, u, since, Some(nonce("4")))
        .await
        .unwrap();
    assert_eq!((other.balance + other.held, other.today), (0, 1));
    let same = credits::usage(&pool, u, since, Some(nonce("3")))
        .await
        .unwrap();
    assert_eq!((same.balance + same.held, same.today), (1, 0));

    // 过期后对账没找到：退回一次；重复关闭不重复退。
    expire_unlanded(&pool, 3).await;
    sp::close(&pool, &[3; 32], SponsorshipStatus::Expired)
        .await
        .unwrap();
    assert_eq!(
        usage(&pool, u).await,
        Usage {
            balance: 1,
            today: 0,
            held: 0
        }
    );
    assert_eq!(
        ledger(&pool, u).await,
        [
            ("starter".to_owned(), 1),
            ("withdraw".to_owned(), -1),
            ("refund_expired".to_owned(), 1),
        ]
    );

    // 作废后经第三方上链：对账找到结果，照收、不退；之后的关闭不动它。
    sign_withdraw(&pool, u, "4", 4).await;
    sp::close(&pool, &[4; 32], SponsorshipStatus::Dropped)
        .await
        .unwrap();
    sp::settle(&pool, outcome(&[4; 32], true)).await.unwrap();
    for status in [SponsorshipStatus::Dropped, SponsorshipStatus::Expired] {
        sp::close(&pool, &[4; 32], status).await.unwrap();
    }
    assert_eq!(usage(&pool, u).await, dropped);
}
