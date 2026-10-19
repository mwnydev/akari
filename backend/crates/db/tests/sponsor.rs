//! forum-sponsor 的代付记录：待上链、免费额度计数、入账幂等、失败暂停、告警去重。需要 PostgreSQL。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

use forum_db::sponsor::{
    self as db, FreeCalls, NewCall, NewSponsorship, Outcome, SponsorshipStatus,
};
use forum_db::{PgPool, users};
use serde_json::json;
use time::{Duration, OffsetDateTime};

const CHAIN: i64 = 31_337;
const SENDER: [u8; 20] = [5; 20];
const ESCROW: [u8; 20] = [2; 20];

async fn sign(pool: &PgPool, user_id: i64, nonce: &str, hash: u8, valid_for: Duration) -> i64 {
    db::insert(
        pool,
        NewSponsorship {
            user_id,
            chain_id: CHAIN,
            sender: &SENDER,
            nonce,
            user_op_hash: &[hash; 32],
            fee: "0",
            valid_until: OffsetDateTime::now_utc() + valid_for,
            signed_block: 1,
        },
    )
    .await
    .unwrap()
    .unwrap()
}

async fn mark_delivered(pool: &PgPool, id: i64, order: &str) {
    db::insert_call(
        pool,
        id,
        NewCall {
            contract: &ESCROW,
            order_id: order,
            method: "markDelivered",
            free: true,
        },
    )
    .await
    .unwrap();
}

fn delivered_calls(order: &str) -> FreeCalls<'_> {
    FreeCalls {
        chain_id: CHAIN,
        contract: &ESCROW,
        order_id: order,
        method: "markDelivered",
        sender: None,
        since: None,
    }
}

async fn user(pool: &PgPool) -> i64 {
    users::upsert_by_email(pool, "s@example.com")
        .await
        .unwrap()
        .0
        .id
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn pending_and_replacement(pool: PgPool) {
    let u = user(&pool).await;
    let a = sign(&pool, u, "7", 1, Duration::minutes(2)).await;
    let pending = db::pending_for_sender(&pool, CHAIN, &SENDER).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!((pending[0].id, pending[0].nonce.as_str()), (a, "7"));

    // 同一 nonce 重签：旧的变 replaced，新的保留
    let b = sign(&pool, u, "7", 2, Duration::minutes(2)).await;
    assert_eq!(
        db::replace_signed(&pool, CHAIN, &SENDER, "7", &[2; 32])
            .await
            .unwrap(),
        1
    );
    let pending = db::pending_for_sender(&pool, CHAIN, &SENDER).await.unwrap();
    assert_eq!(pending.iter().map(|p| p.id).collect::<Vec<_>>(), [b]);

    // 完全相同的签名不重复写入
    let dup = db::insert(
        &pool,
        NewSponsorship {
            user_id: u,
            chain_id: CHAIN,
            sender: &SENDER,
            nonce: "7",
            user_op_hash: &[2; 32],
            fee: "0",
            valid_until: OffsetDateTime::now_utc(),
            signed_block: 1,
        },
    )
    .await
    .unwrap();
    assert_eq!(dup, None);

    // 提交后仍算待上链，即使签名已过期（等对账）
    assert!(
        db::mark_submitted(&pool, b, &json!({}), false)
            .await
            .unwrap()
    );
    assert!(
        !db::mark_submitted(&pool, b, &json!({}), false)
            .await
            .unwrap(),
        "only once"
    );
    sqlx::query("UPDATE sponsorships SET valid_until = now() - interval '1 hour'")
        .execute(&pool)
        .await
        .unwrap();
    let pending = db::pending_for_sender(&pool, CHAIN, &SENDER).await.unwrap();
    assert_eq!(pending[0].status, SponsorshipStatus::Submitted);
    let due = db::due_for_reconcile(&pool, 30.0, 10).await.unwrap();
    assert_eq!(
        due.len(),
        2,
        "replaced and submitted rows are both reconciled"
    );
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn free_calls_count_live_and_settled_but_not_expired(pool: PgPool) {
    let u = user(&pool).await;
    let a = sign(&pool, u, "1", 1, Duration::minutes(2)).await;
    mark_delivered(&pool, a, "9").await;
    assert_eq!(
        db::count_free_calls(&pool, delivered_calls("9"))
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        db::count_free_calls(&pool, delivered_calls("10"))
            .await
            .unwrap(),
        0
    );
    let other_side = FreeCalls {
        sender: Some(&[6; 20]),
        ..delivered_calls("9")
    };
    assert_eq!(db::count_free_calls(&pool, other_side).await.unwrap(), 0);
    let later = FreeCalls {
        since: Some(OffsetDateTime::now_utc() + Duration::minutes(1)),
        ..delivered_calls("9")
    };
    assert_eq!(db::count_free_calls(&pool, later).await.unwrap(), 0);

    // 签名过期未上链：额度释放
    db::close(&pool, &[1; 32], SponsorshipStatus::Expired)
        .await
        .unwrap();
    assert_eq!(
        db::count_free_calls(&pool, delivered_calls("9"))
            .await
            .unwrap(),
        0
    );

    // 上链失败也占额度（平台付了 gas）
    let b = sign(&pool, u, "2", 2, Duration::minutes(2)).await;
    mark_delivered(&pool, b, "9").await;
    let outcome = Outcome {
        user_op_hash: &[2; 32],
        success: false,
        tx_hash: &[9; 32],
        block_number: 5,
        actual_gas_cost: "123",
    };
    assert_eq!(db::settle(&pool, outcome).await.unwrap(), Some(u));
    assert_eq!(
        db::settle(&pool, outcome).await.unwrap(),
        None,
        "settled once"
    );
    db::close(&pool, &[2; 32], SponsorshipStatus::Expired)
        .await
        .unwrap();
    let row = db::by_hash(&pool, &[2; 32]).await.unwrap().unwrap();
    assert_eq!(
        row.status,
        SponsorshipStatus::Failed,
        "close never overrides an outcome"
    );
    assert_eq!(
        db::count_free_calls(&pool, delivered_calls("9"))
            .await
            .unwrap(),
        1
    );
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn third_failure_suspends(pool: PgPool) {
    let u = user(&pool).await;
    for i in 1..=3 {
        let mut tx = pool.begin().await.unwrap();
        let s = db::record_failure(&mut tx, u, 3, 86_400.0, 86_400.0)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(s.is_some(), i == 3, "failure {i}");
    }
    let until = db::suspended_until(&pool, u).await.unwrap().unwrap();
    assert!(until > OffsetDateTime::now_utc() + Duration::hours(23));

    // 窗口外的旧失败不累计
    sqlx::query("UPDATE sponsor_accounts SET suspended_until = NULL, failures = 2, first_failure_at = now() - interval '2 days'")
        .execute(&pool)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    assert!(
        db::record_failure(&mut tx, u, 3, 86_400.0, 86_400.0)
            .await
            .unwrap()
            .is_none()
    );
    tx.commit().await.unwrap();
    assert!(db::suspended_until(&pool, u).await.unwrap().is_none());
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn alerts_are_deduplicated(pool: PgPool) {
    let p = json!({"balance_wei": "1"});
    assert!(
        db::enqueue_alert(&pool, "alert.hot_wallet_low", &p, 3600.0)
            .await
            .unwrap()
    );
    assert!(
        !db::enqueue_alert(&pool, "alert.hot_wallet_low", &p, 3600.0)
            .await
            .unwrap()
    );
    assert!(
        db::enqueue_alert(&pool, "alert.paymaster_deposit_low", &p, 3600.0)
            .await
            .unwrap()
    );
    assert!(
        !db::enqueue_alert(&pool, "email.login_code", &p, 3600.0)
            .await
            .unwrap(),
        "alerts only"
    );
}
