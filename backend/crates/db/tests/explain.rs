//! 执行计划断言（plan 第 12 节「每条新 SQL 附 EXPLAIN 并命中索引」）。
//!
//! 在 5 万主题、一个 2000 楼的热帖、1000 个用户的数据上 `ANALYZE` 后，对 `queries/*.sql`
//! 的**同一份 SQL 文本**做 `EXPLAIN`：大表上不能出现 Seq Scan，且必须用到预期的索引。
//! 会话参数按生产（plan 第 8 节：`random_page_cost = 1.1`）。需要 PostgreSQL（`DATABASE_URL`）。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

use forum_db::PgPool;
use serde_json::{Value, json};
use sqlx::postgres::PgArguments;
use sqlx::query::Query;
use sqlx::{Postgres, Row};

/// 不能被顺序扫描的大表。
const BIG_TABLES: &[&str] = &[
    "topics",
    "posts",
    "post_sources",
    "post_edits",
    "users",
    "outbox",
    "offers",
    "escrow_orders",
];

#[derive(Debug, Default)]
struct PlanInfo {
    seq_scans: Vec<String>,
    indexes: Vec<String>,
}

fn walk(node: &Value, info: &mut PlanInfo) {
    let kind = node["Node Type"].as_str().unwrap_or_default();
    if kind == "Seq Scan"
        && let Some(rel) = node["Relation Name"].as_str()
    {
        info.seq_scans.push(rel.to_owned());
    }
    if let Some(idx) = node["Index Name"].as_str() {
        info.indexes.push(idx.to_owned());
    }
    for child in node["Plans"].as_array().into_iter().flatten() {
        walk(child, info);
    }
}

async fn explain(db: &PgPool, q: Query<'_, Postgres, PgArguments>) -> (PlanInfo, Value) {
    let mut conn = db.acquire().await.unwrap();
    sqlx::query("SET random_page_cost = 1.1")
        .execute(&mut *conn)
        .await
        .unwrap();
    let row = q.fetch_one(&mut *conn).await.unwrap();
    let plan: Value = row.get(0);
    let mut info = PlanInfo::default();
    walk(&plan[0]["Plan"], &mut info);
    (info, plan)
}

/// 断言：大表无顺序扫描，并且用到了 `expect_index`。
async fn check(db: &PgPool, name: &str, q: Query<'_, Postgres, PgArguments>, expect_index: &str) {
    let (info, plan) = explain(db, q).await;
    let bad: Vec<&String> = info
        .seq_scans
        .iter()
        .filter(|r| BIG_TABLES.contains(&r.as_str()))
        .collect();
    assert!(bad.is_empty(), "{name}: seq scan on {bad:?}\n{plan:#}");
    assert!(
        info.indexes.iter().any(|i| i == expect_index),
        "{name}: expected index {expect_index}, used {:?}\n{plan:#}",
        info.indexes
    );
}

async fn seed(db: &PgPool) {
    sqlx::raw_sql(
        r"
        INSERT INTO forums (slug, name) SELECT 'f' || g, 'forum ' || g FROM generate_series(1, 3) g;
        INSERT INTO users (email) SELECT 'u' || g || '@example.com' FROM generate_series(1, 1000) g;
        INSERT INTO user_levels (user_id, level) SELECT id, 2 FROM users WHERE id % 3 = 0;
        -- 5 万主题，分到 3 个板块；1% 置顶、1% 隐藏
        INSERT INTO topics (forum_id, author_id, title, flags, last_floor, last_post_at, created_at)
        SELECT 1 + g % 3, 1 + g % 1000, 'topic ' || g,
               CASE WHEN g % 100 = 0 THEN 1 WHEN g % 100 = 1 THEN 8 ELSE 0 END,
               1, now() - (g || ' seconds')::interval, now() - (g || ' seconds')::interval
        FROM generate_series(1, 50000) g;
        INSERT INTO posts (topic_id, floor, author_id, html, plain_text)
        SELECT id, 1, author_id, '<p>first</p>', 'first ' || id FROM topics;
        -- 一个 2000 楼的热帖
        UPDATE topics SET last_floor = 2000 WHERE id = 2;
        INSERT INTO posts (topic_id, floor, author_id, html, plain_text)
        SELECT 2, g, 1 + g % 1000, '<p>reply</p>', 'reply ' || g FROM generate_series(2, 2000) g;
        INSERT INTO post_sources (post_id, source) SELECT id, '{}'::jsonb FROM posts;
        INSERT INTO post_edits (post_id, editor_id, html, source)
        SELECT 1 + g % 5000, 1, '<p>old</p>', '{}'::jsonb FROM generate_series(1, 20000) g;
        -- 2 万张报价卡片：一半挂在 200 个交易帖下、一半在 2000 个私信会话里；5% 待确认
        INSERT INTO trade_listings (topic_id, seller_id, chain_id, price, terms, content_hash)
        SELECT g, g, 421614, 5000000, '{}'::jsonb, decode(repeat('ab', 32), 'hex') FROM generate_series(1, 200) g;
        INSERT INTO offers (listing_id, conversation_id, seller_id, buyer_id, chain_id, verifying_contract, seller,
          buyer, token, amount, fee_payer, max_fee_bps, max_base_gas, max_min_fee, bond, late_slash_bps, instant,
          ship_window, confirm_window, listing_hash, expiry, max_fills, nonce, signature, offer_hash, status, expires_at)
        SELECT CASE WHEN g % 2 = 0 THEN 1 + g % 200 END, CASE WHEN g % 2 = 1 THEN 1 + g % 2000 END,
               1 + g % 200, CASE WHEN g % 2 = 1 THEN 201 + g % 500 END, 421614,
               decode(repeat('44', 20), 'hex'), decode(repeat('11', 20), 'hex'), decode(repeat('22', 20), 'hex'),
               decode(repeat('33', 20), 'hex'), 5000000, 0, 100, 100000, 1000000, 0, 0, false, 86400, 86400,
               decode(repeat('ab', 32), 'hex'), 1800000000, 1, g, '\x01', sha256(int4send(g)),
               CASE WHEN g % 20 = 0 THEN 'pending' ELSE 'expired' END::offer_status,
               now() + ((g % 100 - 50) || ' hours')::interval
        FROM generate_series(1, 20000) g;
        -- 2 万张订单分给 1000 个买家、1000 个卖家；订单 g 由报价卡片 g 生成
        INSERT INTO chains (chain_id, name, escrow_address, config_address, token_address, token_symbol,
          token_decimals, start_block, safe_depth, finalized_depth)
        VALUES (421614, 'test', decode(repeat('44', 20), 'hex'), decode(repeat('c0', 20), 'hex'),
          decode(repeat('33', 20), 'hex'), 'USDT', 6, 0, 240, 4800);
        INSERT INTO escrow_orders (chain_id, contract, chain_order_id, buyer, seller, buyer_id, seller_id, token,
          amount, instant, offer_hash, listing_hash, state, level, created_block, updated_block)
        SELECT 421614, decode(repeat('44', 20), 'hex'), g, decode(repeat('b1', 20), 'hex'),
               decode(repeat('c1', 20), 'hex'), 1 + g % 1000, 1 + (g + 7) % 1000, decode(repeat('33', 20), 'hex'),
               5000000, false, sha256(int4send(g)), decode(repeat('ab', 32), 'hex'), 1 + g % 5, 'finalized', g, g
        FROM generate_series(1, 20000) g;
        ANALYZE;
        ",
    )
    .execute(db)
    .await
    .unwrap();
}

/// 编译期拼出 `EXPLAIN (FORMAT JSON) <queries/文件 的 SQL>`。
macro_rules! q {
    ($file:literal) => {
        concat!(
            "EXPLAIN (FORMAT JSON) ",
            include_str!(concat!("../queries/", $file))
        )
    };
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn hot_queries_use_indexes(db: PgPool) {
    seed(&db).await;
    let ids: Vec<i64> = (100..150).collect();
    let user_ids: Vec<i64> = (1..=60).collect();
    let before: time::OffsetDateTime =
        sqlx::query_scalar("SELECT now() - interval '20000 seconds'")
            .fetch_one(&db)
            .await
            .unwrap();
    let src = json!({"type": "doc"});

    let sql = q!("topics_by_ids.sql");
    check(
        &db,
        "topics_by_ids",
        sqlx::query(sql).bind(&ids),
        "topics_pkey",
    )
    .await;

    let sql = q!("topics_list_cursor.sql");
    check(
        &db,
        "topics_list_cursor",
        sqlx::query(sql)
            .bind(2_i32)
            .bind(before)
            .bind(i64::MAX)
            .bind(50_i64),
        "topics_forum_list",
    )
    .await;

    let sql = q!("topics_zset_rebuild.sql");
    check(
        &db,
        "topics_zset_rebuild",
        sqlx::query(sql).bind(2_i32).bind(10_000_i64),
        "topics_forum_list",
    )
    .await;

    let sql = q!("topics_pinned.sql");
    check(
        &db,
        "topics_pinned",
        sqlx::query(sql).bind(2_i32),
        "topics_forum_pinned",
    )
    .await;

    // 热帖最后一页与第 1 页一样走 (topic_id, floor) 唯一索引
    let sql = q!("posts_page.sql");
    check(
        &db,
        "posts_page",
        sqlx::query(sql).bind(2_i64).bind(1981_i32).bind(2000_i32),
        "posts_topic_id_floor_key",
    )
    .await;

    let sql = q!("users_cards.sql");
    check(
        &db,
        "users_cards",
        sqlx::query(sql).bind(&user_ids),
        "users_pkey",
    )
    .await;

    let sql = q!("users_actor.sql");
    check(
        &db,
        "users_actor",
        sqlx::query(sql).bind(7_i64),
        "users_pkey",
    )
    .await;

    let sql = q!("posts_by_id.sql");
    check(
        &db,
        "posts_by_id",
        sqlx::query(sql).bind(1234_i64),
        "posts_pkey",
    )
    .await;

    let sql = q!("post_source.sql");
    check(
        &db,
        "post_source",
        sqlx::query(sql).bind(1234_i64),
        "post_sources_pkey",
    )
    .await;

    let sql = q!("post_history.sql");
    check(
        &db,
        "post_history",
        sqlx::query(sql).bind(10_i64).bind(i64::MAX).bind(20_i64),
        "post_edits_post",
    )
    .await;

    // 写语句（EXPLAIN 不执行）：都只按主键定位
    let sql = q!("post_reply.sql");
    check(
        &db,
        "post_reply",
        sqlx::query(sql)
            .bind(2_i64)
            .bind(5_i64)
            .bind("<p>x</p>")
            .bind("x")
            .bind(&src)
            .bind(vec![3_i64])
            .bind(false),
        "topics_pkey",
    )
    .await;

    let sql = q!("topic_create.sql");
    let (info, plan) = explain(
        &db,
        sqlx::query(sql)
            .bind(1_i32)
            .bind(5_i64)
            .bind("t")
            .bind("<p>x</p>")
            .bind("x")
            .bind(&src)
            .bind(vec![3_i64])
            .bind(0_i16),
    )
    .await;
    assert!(info.seq_scans.is_empty(), "topic_create: {plan:#}");

    let sql = q!("offers_by_id.sql");
    check(
        &db,
        "offers_by_id",
        sqlx::query(sql).bind(77_i64),
        "offers_pkey",
    )
    .await;

    let sql = q!("offers_listing.sql");
    check(
        &db,
        "offers_listing",
        sqlx::query(sql).bind(7_i64).bind(Some(8_i64)).bind(50_i64),
        "offers_listing",
    )
    .await;

    let sql = q!("offers_conversation.sql");
    check(
        &db,
        "offers_conversation",
        sqlx::query(sql).bind(7_i64).bind(8_i64).bind(50_i64),
        "offers_conversation",
    )
    .await;

    let sql = q!("offers_expire.sql");
    check(
        &db,
        "offers_expire",
        sqlx::query(sql).bind(500_i64),
        "offers_pending_expiry",
    )
    .await;

    let sql = q!("escrow_orders_buyer.sql");
    check(
        &db,
        "escrow_orders_buyer",
        sqlx::query(sql).bind(7_i64).bind(i64::MAX).bind(20_i64),
        "escrow_orders_buyer",
    )
    .await;

    let sql = q!("escrow_orders_seller.sql");
    check(
        &db,
        "escrow_orders_seller",
        sqlx::query(sql).bind(7_i64).bind(10_000_i64).bind(20_i64),
        "escrow_orders_seller",
    )
    .await;

    let order_ids: Vec<i64> = (100..120).collect();
    let sql = q!("escrow_orders_by_ids.sql");
    check(
        &db,
        "escrow_orders_by_ids",
        sqlx::query(sql).bind(&order_ids),
        "escrow_orders_pkey",
    )
    .await;

    let offer_hash: Vec<u8> = sqlx::query_scalar("SELECT sha256(int4send(77))")
        .fetch_one(&db)
        .await
        .unwrap();
    let sql = q!("escrow_order_by_offer.sql");
    check(
        &db,
        "escrow_order_by_offer",
        sqlx::query(sql)
            .bind(&offer_hash)
            .bind(421_614_i64)
            .bind([0x44_u8; 20].as_slice())
            .bind(78_i64),
        "escrow_orders_offer",
    )
    .await;

    let sql = q!("offer_fills_sync.sql");
    for index in ["escrow_orders_offer", "offers_offer_hash_key"] {
        check(
            &db,
            "offer_fills_sync",
            sqlx::query(sql)
                .bind(421_614_i64)
                .bind([0x44_u8; 20].as_slice())
                .bind(&offer_hash),
            index,
        )
        .await;
    }

    let sql = q!("post_edit_save.sql");
    check(
        &db,
        "post_edit_save",
        sqlx::query(sql)
            .bind(1234_i64)
            .bind(1_i64)
            .bind(None::<String>),
        "posts_pkey",
    )
    .await;

    let sql = q!("post_edit_update.sql");
    check(
        &db,
        "post_edit_update",
        sqlx::query(sql).bind(1234_i64).bind("<p>y</p>").bind("y"),
        "posts_pkey",
    )
    .await;

    let sql = q!("post_source_update.sql");
    check(
        &db,
        "post_source_update",
        sqlx::query(sql).bind(1234_i64).bind(&src),
        "post_sources_pkey",
    )
    .await;

    let sql = q!("post_soft_delete.sql");
    check(
        &db,
        "post_soft_delete",
        sqlx::query(sql).bind(1234_i64).bind(1_i16),
        "posts_pkey",
    )
    .await;

    let sql = q!("topic_update_flags.sql");
    check(
        &db,
        "topic_update_flags",
        sqlx::query(sql).bind(77_i64).bind(1_i16).bind(0_i16),
        "topics_pkey",
    )
    .await;

    let sql = q!("topic_move.sql");
    check(
        &db,
        "topic_move",
        sqlx::query(sql).bind(77_i64).bind(3_i32),
        "topics_pkey",
    )
    .await;

    let sql = q!("topic_set_title.sql");
    check(
        &db,
        "topic_set_title",
        sqlx::query(sql).bind(77_i64).bind("t2"),
        "topics_pkey",
    )
    .await;

    let sql = q!("topic_add_views.sql");
    check(
        &db,
        "topic_add_views",
        sqlx::query(sql).bind(&ids).bind(vec![1_i64; ids.len()]),
        "topics_pkey",
    )
    .await;
}

/// 提币地址：1000 个用户各有 5 个地址（其中 3 个已撤销）。代付检查、列表与添加前检查都按用户走索引。
#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn withdraw_address_queries_use_indexes(db: PgPool) {
    sqlx::raw_sql(
        r"
        INSERT INTO users (email) SELECT 'u' || g || '@example.com' FROM generate_series(1, 1000) g;
        INSERT INTO withdraw_addresses (user_id, chain_id, address, label, active_at, revoked_at)
        SELECT 1 + g % 1000, 421614, decode(lpad(to_hex(g), 40, '0'), 'hex'), '', now(),
               CASE WHEN g % 5 < 3 THEN now() END
        FROM generate_series(1, 5000) g;
        ANALYZE;
        ",
    )
    .execute(&db)
    .await
    .unwrap();
    let addr = vec![0x11_u8; 20];
    let cases = [
        (
            "withdraw_addresses_active",
            sqlx::query(q!("withdraw_addresses_active.sql"))
                .bind(7_i64)
                .bind(421614_i64),
            "withdraw_addresses_live",
        ),
        (
            "withdraw_addresses_list",
            sqlx::query(q!("withdraw_addresses_list.sql"))
                .bind(7_i64)
                .bind(421614_i64)
                .bind(forum_db::withdraw::LIST_LIMIT),
            "withdraw_addresses_user",
        ),
        (
            "withdraw_addresses_stats",
            sqlx::query(q!("withdraw_addresses_stats.sql"))
                .bind(7_i64)
                .bind(421614_i64)
                .bind(addr),
            "withdraw_addresses_live",
        ),
    ];
    for (name, q, index) in cases {
        let (info, plan) = explain(&db, q).await;
        assert!(
            !info.seq_scans.iter().any(|t| t == "withdraw_addresses"),
            "{name}: seq scan\n{plan:#}"
        );
        assert!(
            info.indexes.iter().any(|i| i == index),
            "{name}: expected index {index}, used {:?}\n{plan:#}",
            info.indexes
        );
    }
}
