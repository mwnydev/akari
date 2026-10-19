//! 链上派生表的 SQL 语义（需要 PostgreSQL，见 tests/db.rs）。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

use forum_db::PgPool;
use forum_db::chain::{self, ChainRow, NewEvent, OrderUpsert};
use serde_json::json;

const CHAIN: i64 = 31_337;
const ESCROW: [u8; 20] = [0xe5; 20];

async fn setup(pool: &PgPool) {
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

fn event<'a>(
    block: i64,
    hash: &'a [u8],
    tx: &'a [u8],
    payload: &'a serde_json::Value,
) -> NewEvent<'a> {
    NewEvent {
        chain_id: CHAIN,
        contract: &ESCROW,
        block_number: block,
        block_hash: hash,
        block_time: None,
        tx_hash: tx,
        log_index: 0,
        name: "Delivered",
        order_id: Some(1),
        topics: &[],
        data: &[],
        payload,
    }
}

async fn block_of(pool: &PgPool, tx: &[u8]) -> (i64, String) {
    sqlx::query_as("SELECT block_number, level::text FROM escrow_events WHERE tx_hash = $1")
        .bind(tx)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn same_log_moves_block_only_when_replacing(pool: PgPool) {
    setup(&pool).await;
    let (tx, p) = ([1u8; 32], json!({}));
    assert!(
        chain::insert_event(&pool, event(10, &[0xa; 32], &tx, &p), false)
            .await
            .unwrap()
    );
    // 同一区块再写：无改动。
    assert!(
        !chain::insert_event(&pool, event(10, &[0xa; 32], &tx, &p), true)
            .await
            .unwrap()
    );
    // WebSocket（不可靠）推送了另一个区块里的同一日志：保留原行。
    assert!(
        !chain::insert_event(&pool, event(12, &[0xb; 32], &tx, &p), false)
            .await
            .unwrap()
    );
    assert_eq!(block_of(&pool, &tx).await.0, 10);
    // 校对拿到的主链数据：改到新区块。
    assert!(
        chain::insert_event(&pool, event(12, &[0xb; 32], &tx, &p), true)
            .await
            .unwrap()
    );
    assert_eq!(block_of(&pool, &tx).await.0, 12);
    // 最终确认后永远不改。
    let touched = chain::promote(&pool, CHAIN, 12, "finalized").await.unwrap();
    assert_eq!(touched, vec![(ESCROW.to_vec(), 1)]);
    assert!(
        !chain::insert_event(&pool, event(13, &[0xc; 32], &tx, &p), true)
            .await
            .unwrap()
    );
    assert_eq!(block_of(&pool, &tx).await, (12, "finalized".to_owned()));
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn rollback_keeps_finalized_rows_and_reports_orders(pool: PgPool) {
    setup(&pool).await;
    let p = json!({});
    for (block, tx) in [(5, [5u8; 32]), (8, [8u8; 32]), (9, [9u8; 32])] {
        chain::insert_event(&pool, event(block, &[block as u8; 32], &tx, &p), true)
            .await
            .unwrap();
        chain::set_cursor(&pool, CHAIN, block, &[block as u8; 32])
            .await
            .unwrap();
    }
    chain::promote(&pool, CHAIN, 5, "finalized").await.unwrap();
    chain::promote(&pool, CHAIN, 8, "safe").await.unwrap();
    assert_eq!(block_of(&pool, &[8u8; 32]).await.1, "safe");

    let touched = chain::rollback_after(&pool, CHAIN, 4).await.unwrap();
    assert_eq!(touched, vec![(ESCROW.to_vec(), 1)]);
    let left: Vec<i64> = sqlx::query_scalar("SELECT block_number FROM escrow_events ORDER BY 1")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(
        left,
        vec![5],
        "finalized row survives even below the fork point"
    );
    let cps: Vec<i64> = chain::checkpoints(&pool, CHAIN)
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.0)
        .collect();
    assert!(
        cps.is_empty(),
        "checkpoints after the fork removed: {cps:?}"
    );
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn confirmations_prune_old_checkpoints(pool: PgPool) {
    setup(&pool).await;
    for n in [10, 20, 30, 40] {
        chain::set_cursor(&pool, CHAIN, n, &[n as u8; 32])
            .await
            .unwrap();
    }
    chain::set_confirmations(&pool, CHAIN, 35, 25)
        .await
        .unwrap();
    let mut cps: Vec<i64> = chain::checkpoints(&pool, CHAIN)
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.0)
        .collect();
    cps.sort_unstable();
    assert_eq!(
        cps,
        vec![20, 30, 40],
        "keeps the newest checkpoint at or before finality"
    );
    let c = chain::cursor(&pool, CHAIN).await.unwrap().unwrap();
    assert_eq!(
        (c.block_number, c.safe_block, c.finalized_block),
        (40, 35, 25)
    );
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn order_upsert_reports_only_real_changes(pool: PgPool) {
    setup(&pool).await;
    let mut o = OrderUpsert {
        chain_id: CHAIN,
        contract: &ESCROW,
        chain_order_id: 1,
        buyer: vec![0xb1; 20],
        seller: vec![0xc1; 20],
        token: vec![0xd1; 20],
        amount: "100000000000000000000000000000000000".into(),
        offer_hash: vec![1; 32],
        listing_hash: vec![2; 32],
        state: 1,
        level: "latest",
        created_block: 3,
        updated_block: 3,
        ..Default::default()
    };
    let first = chain::upsert_order(&pool, &o)
        .await
        .unwrap()
        .expect("inserted");
    assert!(
        chain::upsert_order(&pool, &o).await.unwrap().is_none(),
        "unchanged"
    );
    o.state = 3;
    o.settled_to_seller = Some("99".into());
    assert_eq!(
        chain::upsert_order(&pool, &o).await.unwrap().map(|c| c.id),
        Some(first.id)
    );
    let amount: String = sqlx::query_scalar("SELECT amount::text FROM escrow_orders WHERE id = $1")
        .bind(first.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(amount, o.amount, "numeric(38,0) keeps every digit");

    assert!(
        chain::clear_order(&pool, CHAIN, &ESCROW, 1)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        chain::clear_order(&pool, CHAIN, &ESCROW, 1)
            .await
            .unwrap()
            .is_none(),
        "already cleared"
    );
}

/// 生成订单的报价卡片：成交次数用完即已付款；区块回滚撤销订单后恢复。
#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn offer_paid_follows_order_fills(pool: PgPool) {
    setup(&pool).await;
    let seller: i64 =
        sqlx::query_scalar("INSERT INTO users (email) VALUES ('s@example.com') RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap();
    let offer_hash = [7u8; 32];
    // 可成交 2 次的报价。
    let twice: i64 = sqlx::query_scalar(
            "INSERT INTO offers (conversation_id, seller_id, chain_id, verifying_contract, seller, buyer, token,
               amount, fee_payer, max_fee_bps, max_base_gas, max_min_fee, bond, late_slash_bps, instant, ship_window,
               confirm_window, listing_hash, expiry, max_fills, nonce, signature, offer_hash, expires_at)
             VALUES (1, $1, $2, $3, $4, $4, $4, 5000000, 0, 100, 100000, 1000000, 0, 0, false, 86400, 86400,
               $5, 1800000000, 2, 1, '\\x01', $6, now() + interval '1 day')
         RETURNING id",
    )
    .bind(seller)
    .bind(CHAIN)
    .bind(ESCROW.as_slice())
    .bind([0u8; 20].as_slice())
    .bind([2u8; 32].as_slice())
    .bind(offer_hash.as_slice())
    .fetch_one(&pool)
    .await
    .unwrap();
    let status = |id: i64| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, String>("SELECT status::text FROM offers WHERE id = $1")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    let order = |chain_order_id: i64| OrderUpsert {
        chain_id: CHAIN,
        contract: &ESCROW,
        chain_order_id,
        buyer: vec![0xb1; 20],
        seller: vec![0xc1; 20],
        token: vec![0xd1; 20],
        amount: "5000000".into(),
        offer_hash: offer_hash.to_vec(),
        listing_hash: vec![2; 32],
        state: 1,
        level: "latest",
        created_block: 3,
        updated_block: 3,
        ..Default::default()
    };
    let sync = || chain::sync_offer_fills(&pool, CHAIN, &ESCROW, &offer_hash);

    chain::upsert_order(&pool, &order(1)).await.unwrap();
    assert!(
        !sync().await.unwrap(),
        "one of two fills used: still pending"
    );
    assert_eq!(status(twice).await, "pending");
    chain::upsert_order(&pool, &order(2)).await.unwrap();
    assert!(sync().await.unwrap());
    assert_eq!(status(twice).await, "paid");
    assert!(!sync().await.unwrap(), "already paid: no rewrite");

    // 回滚撤销第 2 单：恢复为待确认。
    chain::clear_order(&pool, CHAIN, &ESCROW, 2).await.unwrap();
    assert!(sync().await.unwrap());
    assert_eq!(status(twice).await, "pending");

    // 另一份合约上同哈希的订单不算。
    let other = OrderUpsert {
        contract: &[0x5e; 20],
        ..order(2)
    };
    chain::upsert_order(&pool, &other).await.unwrap();
    assert!(!sync().await.unwrap());
    assert_eq!(status(twice).await, "pending");
}
