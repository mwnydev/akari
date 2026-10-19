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
    "conversations",
    "messages",
    "order_messages",
    "exp_ledger",
    "uploads",
    "user_trade_stats",
    "post_likes",
    "post_counters",
    "floor_comments",
    "reports",
    "report_cases",
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
    check_any(db, name, q, &[expect_index]).await;
}

/// 同 [`check`]，用到其中任一索引即可（等价的索引由规划器按统计信息选）。
async fn check_any(
    db: &PgPool,
    name: &str,
    q: Query<'_, Postgres, PgArguments>,
    expect_any: &[&str],
) {
    let (info, plan) = explain(db, q).await;
    // 有数据的通知月分区同样不能顺序扫描（未来月份的空分区扫一下没有代价）。
    let partitions: Vec<String> = sqlx::query_scalar(
        "SELECT c.relname::text FROM pg_inherits i JOIN pg_class c ON c.oid = i.inhrelid
         WHERE i.inhparent = 'notifications'::regclass AND c.reltuples > 0",
    )
    .fetch_all(db)
    .await
    .unwrap();
    let bad: Vec<&String> = info
        .seq_scans
        .iter()
        .filter(|r| BIG_TABLES.contains(&r.as_str()) || partitions.contains(r))
        .collect();
    assert!(bad.is_empty(), "{name}: seq scan on {bad:?}\n{plan:#}");
    assert!(
        info.indexes
            .iter()
            .any(|i| expect_any.contains(&i.as_str())),
        "{name}: expected one of {expect_any:?}, used {:?}\n{plan:#}",
        info.indexes
    );
}

async fn seed(db: &PgPool) {
    sqlx::raw_sql(
        r"
        INSERT INTO forums (slug, name) SELECT 'f' || g, 'forum ' || g FROM generate_series(1, 3) g;
        INSERT INTO users (email) SELECT 'u' || g || '@example.com' FROM generate_series(1, 1000) g;
        INSERT INTO user_levels (user_id, level) SELECT id, 2 FROM users WHERE id % 3 = 0;
        -- 另 2 万个有交易信誉的用户；9 成用户设置了用户名
        INSERT INTO users (email) SELECT 'v' || g || '@example.com' FROM generate_series(1, 20000) g;
        UPDATE users SET username = 'user_' || id WHERE id % 10 <> 0;
        INSERT INTO user_trade_stats (user_id, success) SELECT id, id % 7 FROM users WHERE id % 2 = 0;
        -- 2 万条经验流水、2 万张上传（1% 待处理）
        INSERT INTO exp_ledger (user_id, reason, amount, ref_id, day, dedupe)
        SELECT 1 + g % 1000, 1 + g % 5, 2, g, current_date - (g % 30), 'post:' || g FROM generate_series(1, 20000) g;
        INSERT INTO uploads (user_id, status, content_type, size, tmp_key)
        SELECT 1 + g % 1000, CASE WHEN g % 100 = 0 THEN 'queued' ELSE 'done' END::upload_status, 'image/jpeg', 1000,
               'tmp/' || g FROM generate_series(1, 20000) g;

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
        -- 2000 个私信会话（用户 1–400 各与 5 个人），20 万条消息；会话的最后一条与已读位置按消息回填
        INSERT INTO conversations (user_low, user_high)
        SELECT 1 + (g - 1) % 400, 401 + ((g - 1) / 400) * 100 + (g - 1) % 100 FROM generate_series(1, 2000) g;
        INSERT INTO messages (conversation_id, sender_id, kind, body)
        SELECT c.id, CASE WHEN g % 2 = 0 THEN c.user_low ELSE c.user_high END, 'text', 'message ' || g
        FROM generate_series(1, 200000) g JOIN conversations c ON c.id = 1 + g % 2000;
        UPDATE conversations c SET last_message_id = m.last, low_read_id = m.last - 500, high_read_id = m.last
        FROM (SELECT conversation_id, max(id) AS last FROM messages GROUP BY conversation_id) m
        WHERE m.conversation_id = c.id;
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
        -- 订单聊天：每单 3 条（哈希只需满足长度与唯一约束）
        INSERT INTO order_messages (order_id, sender_id, kind, body, prev_hash, hash, created_at)
        SELECT 1 + g % 20000, NULL, 'system', 'm' || g, sha256(int8send(g)), sha256(int8send(-g)), now()
        FROM generate_series(1, 60000) g;
        -- 通知：再建两个月前的分区，20 万条分给 1000 个用户、分布在最近 60 天
        SELECT notifications_maintain(now() - interval '2 months');
        INSERT INTO notifications (user_id, kind, payload, created_at)
        SELECT 1 + g % 1000, 'reply', '{}'::jsonb, now() - ((g % 60) || ' days')::interval
        FROM generate_series(1, 200000) g;
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

    let sql = q!("conversations_list.sql");
    for index in [
        "conversations_low_list",
        "conversations_high_list",
        "messages_pkey",
        "messages_conversation",
    ] {
        check(
            &db,
            "conversations_list",
            sqlx::query(sql).bind(7_i64).bind(i64::MAX).bind(20_i64),
            index,
        )
        .await;
    }

    let sql = q!("messages_before.sql");
    check(
        &db,
        "messages_before",
        sqlx::query(sql).bind(77_i64).bind(i64::MAX).bind(51_i64),
        "messages_conversation",
    )
    .await;
    let sql = q!("messages_after.sql");
    check(
        &db,
        "messages_after",
        sqlx::query(sql).bind(77_i64).bind(1000_i64).bind(51_i64),
        "messages_conversation",
    )
    .await;

    let sql = q!("order_messages_before.sql");
    check(
        &db,
        "order_messages_before",
        sqlx::query(sql).bind(77_i64).bind(i64::MAX).bind(51_i64),
        "order_messages_order",
    )
    .await;
    let sql = q!("order_messages_after.sql");
    check(
        &db,
        "order_messages_after",
        sqlx::query(sql).bind(77_i64).bind(0_i64).bind(1000_i64),
        "order_messages_order",
    )
    .await;

    // 每个分区上的 (user_id, id DESC) 索引；本月分区的索引名
    let notif_index: String = sqlx::query_scalar(
        "SELECT 'notifications_' || to_char(now() AT TIME ZONE 'UTC', 'YYYYMM') || '_user_id_id_idx'",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    let sql = q!("notifications_before.sql");
    check(
        &db,
        "notifications_before",
        sqlx::query(sql)
            .bind(7_i64)
            .bind(i64::MAX)
            .bind(20_i64)
            .bind(0_i64),
        &notif_index,
    )
    .await;
    let sql = q!("notifications_after.sql");
    check(
        &db,
        "notifications_after",
        sqlx::query(sql)
            .bind(7_i64)
            .bind(100_000_i64)
            .bind(20_i64)
            .bind(0_i64),
        &notif_index,
    )
    .await;
    let sql = q!("notifications_unread.sql");
    check(
        &db,
        "notifications_unread",
        sqlx::query(sql).bind(7_i64).bind(150_000_i64).bind(100_i64),
        &notif_index,
    )
    .await;

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

    // ───── 第 5 轮论坛缺口：交易帖价格、跨板块最新、个人主页、名片、信誉、经验、图片上传 ─────

    let sql = q!("topics_latest_rebuild.sql");
    check(
        &db,
        "topics_latest_rebuild",
        sqlx::query(sql).bind(10_000_i64),
        "topics_latest",
    )
    .await;

    let sql = q!("topics_latest_cursor.sql");
    check(
        &db,
        "topics_latest_cursor",
        sqlx::query(sql)
            .bind(vec![1_i32, 2, 3])
            .bind(before)
            .bind(i64::MAX)
            .bind(50_i64),
        "topics_latest",
    )
    .await;

    let sql = q!("users_search.sql");
    check(
        &db,
        "users_search",
        sqlx::query(sql).bind("User_12").bind(10_i64),
        "users_username_prefix",
    )
    .await;

    // 小写用户名上的两个索引都能做等值查找
    let sql = q!("users_id_by_name.sql");
    check_any(
        &db,
        "users_id_by_name",
        sqlx::query(sql).bind("USER_77"),
        &["users_username", "users_username_prefix"],
    )
    .await;

    let sql = q!("users_profile.sql");
    check(
        &db,
        "users_profile",
        sqlx::query(sql).bind(7_i64),
        "users_pkey",
    )
    .await;

    let sql = q!("username_set.sql");
    check(
        &db,
        "username_set",
        sqlx::query(sql).bind(7_i64).bind("new_name"),
        "users_pkey",
    )
    .await;

    let sql = q!("user_posts.sql");
    check(
        &db,
        "user_posts",
        sqlx::query(sql)
            .bind(5_i64)
            .bind(i64::MAX)
            .bind(vec![1_i32, 2, 3])
            .bind(20_i64),
        "posts_author",
    )
    .await;

    let sql = q!("user_post_counts.sql");
    check(
        &db,
        "user_post_counts",
        sqlx::query(sql).bind(5_i64),
        "posts_author",
    )
    .await;

    for index in ["escrow_orders_buyer", "escrow_orders_seller"] {
        check(
            &db,
            "user_trade_stats_refresh",
            sqlx::query(q!("user_trade_stats_refresh.sql"))
                .bind(vec![7_i64, 8])
                .bind(10_i32),
            index,
        )
        .await;
    }

    let sql = q!("trade_outcomes_by_ids.sql");
    check(
        &db,
        "trade_outcomes_by_ids",
        sqlx::query(sql).bind(&order_ids).bind(20_i32),
        "escrow_orders_pkey",
    )
    .await;

    let today: time::Date = sqlx::query_scalar("SELECT current_date")
        .fetch_one(&db)
        .await
        .unwrap();
    let sql = q!("exp_insert.sql");
    check(
        &db,
        "exp_insert",
        sqlx::query(sql)
            .bind(7_i64)
            .bind(1_i16)
            .bind(2_i32)
            .bind(Some(99_i64))
            .bind(today)
            .bind(Some("post:99"))
            .bind(Some(5_i32)),
        "exp_ledger_user_day",
    )
    .await;

    let sql = q!("exp_net.sql");
    check(
        &db,
        "exp_net",
        sqlx::query(sql).bind(7_i64).bind(3_i16).bind(77_i64),
        "exp_ledger_ref",
    )
    .await;

    let sql = q!("exp_lock.sql");
    check(
        &db,
        "exp_lock",
        sqlx::query(sql).bind(9_i64),
        "user_levels_pkey",
    )
    .await;

    let sql = q!("exp_apply.sql");
    check(
        &db,
        "exp_apply",
        sqlx::query(sql).bind(9_i64).bind(2_i64),
        "user_levels_pkey",
    )
    .await;

    let sql = q!("exp_posts_due.sql");
    check(
        &db,
        "exp_posts_due",
        sqlx::query(sql).bind(40_000_i64).bind(500_i64),
        "posts_pkey",
    )
    .await;

    // job_cursor_lock：job_cursors 每个后台任务一行，不需要计划断言。

    let sql = q!("upload_claim.sql");
    check(
        &db,
        "upload_claim",
        sqlx::query(sql).bind(120.0_f64),
        "uploads_due",
    )
    .await;

    let sql = q!("uploads_by_id.sql");
    check(
        &db,
        "uploads_by_id",
        sqlx::query(sql).bind(77_i64),
        "uploads_pkey",
    )
    .await;

    let sql = q!("upload_queue.sql");
    check_any(
        &db,
        "upload_queue",
        sqlx::query(sql).bind(77_i64).bind(78_i64),
        &["uploads_pkey", "uploads_user"],
    )
    .await;

    // 处理中的行也在部分索引 uploads_due 里，按主键定位哪个都行
    let due_or_pkey = ["uploads_pkey", "uploads_due"];
    let sql = q!("upload_done.sql");
    check_any(
        &db,
        "upload_done",
        sqlx::query(sql)
            .bind(77_i64)
            .bind(10_i32)
            .bind(10_i32)
            .bind(json!([])),
        &due_or_pkey,
    )
    .await;

    for name in ["upload_failed.sql", "upload_retry_later.sql"] {
        let q = match name {
            "upload_failed.sql" => sqlx::query(q!("upload_failed.sql"))
                .bind(77_i64)
                .bind("corrupt"),
            _ => sqlx::query(q!("upload_retry_later.sql"))
                .bind(77_i64)
                .bind(30.0_f64)
                .bind("storage"),
        };
        check_any(&db, name, q, &due_or_pkey).await;
    }
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

/// 代付提币次数：1000 个用户、5 万笔代付签名（其中 2 万笔是提币，大多已结束），流水 3 万多行。
/// 余额按主键读，当天次数按 (user_id, created_at) 部分索引，同 nonce 旧签名按待上链部分索引。
#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn withdraw_credit_usage_uses_indexes(db: PgPool) {
    sqlx::raw_sql(
        r"
        INSERT INTO users (email) SELECT 'u' || g || '@example.com' FROM generate_series(1, 1000) g;
        INSERT INTO sponsorships (user_id, chain_id, sender, nonce, user_op_hash, fee, valid_until, signed_block,
                                  status, created_at)
        SELECT 1 + g % 1000, 421614, decode(lpad(to_hex(1 + g % 1000), 40, '0'), 'hex'), g / 1000,
               sha256(int4send(g)), 50000, now() - (g || ' minutes')::interval, g,
               CASE WHEN g % 50 = 0 THEN 'signed' WHEN g % 7 = 0 THEN 'expired' ELSE 'included' END::sponsorship_status,
               now() - (g || ' minutes')::interval
        FROM generate_series(1, 50000) g;
        INSERT INTO withdraw_credit_ledger (user_id, delta, reason) SELECT id, 1, 'starter' FROM users;
        INSERT INTO withdraw_credit_ledger (user_id, delta, reason, sponsorship_id, created_at)
        SELECT user_id, -1, 'withdraw', id, created_at FROM sponsorships WHERE id % 5 < 2;
        INSERT INTO withdraw_credit_ledger (user_id, delta, reason, sponsorship_id, created_at)
        SELECT user_id, 1, 'refund_expired', id, created_at FROM sponsorships WHERE id % 5 < 2 AND status = 'expired';
        ANALYZE;
        ",
    )
    .execute(&db)
    .await
    .unwrap();
    let since: time::OffsetDateTime = sqlx::query_scalar("SELECT now() - interval '1 day'")
        .fetch_one(&db)
        .await
        .unwrap();
    let sender = {
        let mut a = vec![0_u8; 18];
        a.extend_from_slice(&7_u16.to_be_bytes());
        a
    };
    let with_nonce = sqlx::query(q!("withdraw_credits_usage.sql"))
        .bind(7_i64)
        .bind(since)
        .bind(Some(421614_i64))
        .bind(Some(sender))
        .bind(Some("3"));
    let without = sqlx::query(q!("withdraw_credits_usage.sql"))
        .bind(7_i64)
        .bind(since)
        .bind(None::<i64>)
        .bind(None::<Vec<u8>>)
        .bind(None::<String>);
    // 不带 nonce 时同 nonce 那一项恒为 0，计划里不扫 sponsorships 的待上链索引。
    let cases = [
        ("with nonce", with_nonce, true),
        ("balance only", without, false),
    ];
    for (name, q, by_nonce) in cases {
        let (info, plan) = explain(&db, q).await;
        assert!(
            !info
                .seq_scans
                .iter()
                .any(|t| t == "withdraw_credit_ledger" || t == "sponsorships"),
            "{name}: seq scan\n{plan:#}"
        );
        let mut expected = vec!["withdraw_credits_pkey", "withdraw_credit_ledger_withdraws"];
        if by_nonce {
            expected.push("sponsorships_open");
        }
        for index in expected {
            assert!(
                info.indexes.iter().any(|i| i == index),
                "{name}: expected index {index}, used {:?}\n{plan:#}",
                info.indexes
            );
        }
    }
}

/// 管理与处罚（ADR 0013）：2 万用户、3000 条任命与暂停记录、2 万条处罚、5 万条审计日志。
/// 每个管理请求查角色、每次发帖查禁言、每次登录查 IP / 设备封禁、每次删帖 / 处罚做异常计数，都按索引走。
#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn staff_queries_use_indexes(db: PgPool) {
    sqlx::raw_sql(
        r"
        INSERT INTO forums (slug, name) SELECT 'f' || g, 'forum ' || g FROM generate_series(1, 20) g;
        INSERT INTO users (email) SELECT 'u' || g || '@example.com' FROM generate_series(1, 20000) g;
        INSERT INTO staff_roles (user_id, role, forum_id)
        SELECT g, CASE WHEN g % 10 = 0 THEN 3 ELSE 4 END, CASE WHEN g % 10 = 0 THEN NULL ELSE 1 + g % 20 END
        FROM generate_series(1, 3000) g;
        INSERT INTO staff_suspensions (user_id, reason, reinstated_at, reinstated_by)
        SELECT g, 'anomaly', CASE WHEN g % 3 > 0 THEN now() END, CASE WHEN g % 3 > 0 THEN 1 END
        FROM generate_series(1, 3000) g;
        INSERT INTO sanctions (kind, user_id, forum_id, ip, device_hash, ends_at, reason, created_by, created_at, revoked_at,
                               revoked_by)
        SELECT 1 + g % 5,
               CASE WHEN g % 5 <> 3 THEN 1 + g % 20000 END,
               CASE WHEN g % 5 IN (0, 1) AND g % 2 = 0 THEN 1 + g % 20 END,
               CASE WHEN g % 5 = 3 THEN ('10.' || (g % 250) || '.' || (g / 250 % 250) || '.0/24')::cidr END,
               CASE WHEN g % 5 = 4 THEN sha256(int4send(g)) END,
               now() + ((g % 40 - 10) || ' days')::interval, 'x', 1, now() - interval '20 days',
               CASE WHEN g % 4 = 0 THEN now() END, CASE WHEN g % 4 = 0 THEN 1 END
        FROM generate_series(1, 20000) g;
        INSERT INTO admin_audit_logs (actor_id, action, target, created_at)
        SELECT 1 + g % 300, CASE WHEN g % 2 = 0 THEN 'post.delete' ELSE 'topic.pin' END, 'post:' || g,
               now() - (g || ' minutes')::interval
        FROM generate_series(1, 50000) g;
        INSERT INTO admin_audit_logs (actor_id, action, reverts) SELECT 1, 'post.delete.revert', g * 7
        FROM generate_series(1, 2000) g;
        ANALYZE;
        ",
    )
    .execute(&db)
    .await
    .unwrap();
    let kinds: Vec<i16> = vec![1, 3];
    let actions: Vec<String> = vec!["post.delete".into(), "topic.hide".into()];
    let now = time::OffsetDateTime::now_utc();
    let cases: [(&str, Query<'_, Postgres, PgArguments>, &[&str]); 5] = [
        (
            "staff_roles_of",
            sqlx::query(q!("staff_roles_of.sql")).bind(77_i64),
            &["staff_roles_unique"],
        ),
        (
            "sanctions_active_kind",
            sqlx::query(q!("sanctions_active_kind.sql"))
                .bind(77_i64)
                .bind(kinds)
                .bind(Some(3_i32)),
            &["sanctions_user_active", "sanctions_user"],
        ),
        (
            "sanctions_access",
            sqlx::query(q!("sanctions_access.sql"))
                .bind("10.7.3.9")
                .bind(Some(vec![0xab_u8; 32])),
            &["sanctions_ip_active"],
        ),
        (
            "audit_count_recent",
            sqlx::query(q!("audit_count_recent.sql"))
                .bind(77_i64)
                .bind(actions)
                .bind(3600.0_f64),
            &["admin_audit_logs_actor"],
        ),
        (
            "audit_by_actor",
            sqlx::query(q!("audit_by_actor.sql"))
                .bind(77_i64)
                .bind(now - time::Duration::days(7))
                .bind(now)
                .bind(501_i64),
            &["admin_audit_logs_actor"],
        ),
    ];
    for (name, q, expect) in cases {
        let (info, plan) = explain(&db, q).await;
        let bad: Vec<&String> = info
            .seq_scans
            .iter()
            .filter(|t| {
                [
                    "sanctions",
                    "admin_audit_logs",
                    "users",
                    "staff_suspensions",
                ]
                .contains(&t.as_str())
            })
            .collect();
        assert!(bad.is_empty(), "{name}: seq scan on {bad:?}\n{plan:#}");
        assert!(
            info.indexes.iter().any(|i| expect.contains(&i.as_str())),
            "{name}: expected one of {expect:?}, used {:?}\n{plan:#}",
            info.indexes
        );
        if name == "sanctions_access" {
            assert!(
                info.indexes.iter().any(|i| i == "sanctions_device_active"),
                "{name}: {plan:#}"
            );
        }
    }
}

/// 楼层点赞与评论（设计稿 v2）：在热帖每楼 50 个赞、10 条评论（1 条已删除）、其余 5 万个 1 楼各 2 个赞 1 条评论的数据上。
#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn floor_social_queries_use_indexes(db: PgPool) {
    seed(&db).await;
    sqlx::raw_sql(
        r"
        INSERT INTO post_likes (post_id, user_id, created_at)
        SELECT p.id, 1 + (p.id * 7 + g) % 1000, now() - (g || ' minutes')::interval
        FROM posts p, generate_series(1, 50) g WHERE p.topic_id = 2;
        INSERT INTO post_likes (post_id, user_id) SELECT p.id, 1 + g FROM posts p, generate_series(1, 2) g WHERE p.topic_id <> 2;
        INSERT INTO floor_comments (post_id, author_id, body, status)
        SELECT p.id, 1 + g % 1000, 'c' || g, CASE WHEN g = 10 THEN 1 ELSE 0 END
        FROM posts p, generate_series(1, 10) g WHERE p.topic_id = 2;
        INSERT INTO floor_comments (post_id, author_id, body) SELECT id, author_id, 'c' FROM posts WHERE topic_id <> 2;
        INSERT INTO post_counters (post_id, likes, comments)
        SELECT p.id, (SELECT count(*) FROM post_likes l WHERE l.post_id = p.id),
               (SELECT count(*) FROM floor_comments c WHERE c.post_id = p.id AND c.status = 0)
        FROM posts p;
        INSERT INTO report_cases (target_kind, target_id, forum_id, target_author_id)
        SELECT 1, g, 1, 1 + g % 1000 FROM generate_series(1, 20000) g;
        INSERT INTO reports (target_kind, target_id, forum_id, reporter_id, case_id)
        SELECT 1, g, 1, 1 + g % 1000, g FROM generate_series(1, 20000) g;
        ANALYZE;
        ",
    )
    .execute(&db)
    .await
    .unwrap();
    // 热帖第 50 页的 20 个楼层
    let post_ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM posts WHERE topic_id = 2 AND floor BETWEEN 981 AND 1000",
    )
    .fetch_all(&db)
    .await
    .unwrap();
    let post = post_ids[3];

    let sql = q!("posts_social.sql");
    for index in ["post_counters_pkey", "post_likes_recent", "post_likes_pkey"] {
        check(
            &db,
            "posts_social",
            sqlx::query(sql).bind(&post_ids).bind(7_i64).bind(5_i64),
            index,
        )
        .await;
    }

    let sql = q!("comments_preview.sql");
    check(
        &db,
        "comments_preview",
        sqlx::query(sql).bind(&post_ids).bind(5_i64),
        "floor_comments_post",
    )
    .await;

    let sql = q!("comments_after.sql");
    check(
        &db,
        "comments_after",
        sqlx::query(sql).bind(post).bind(0_i64).bind(51_i64),
        "floor_comments_post",
    )
    .await;

    let sql = q!("comment_by_id.sql");
    check(
        &db,
        "comment_by_id",
        sqlx::query(sql).bind(1234_i64),
        "floor_comments_pkey",
    )
    .await;

    // 写语句（EXPLAIN 不执行）：楼层、计数行、评论都按主键 / 唯一索引定位
    let sql = q!("post_like.sql");
    for index in ["posts_pkey", "post_counters_pkey"] {
        check(
            &db,
            "post_like",
            sqlx::query(sql)
                .bind(post)
                .bind(9_i64)
                .bind(true)
                .bind(300.0_f64),
            index,
        )
        .await;
    }

    let sql = q!("post_unlike.sql");
    for index in ["post_likes_pkey", "post_counters_pkey"] {
        check(
            &db,
            "post_unlike",
            sqlx::query(sql).bind(post).bind(9_i64),
            index,
        )
        .await;
    }

    let sql = q!("comment_create.sql");
    for index in ["posts_pkey", "floor_comments_pkey"] {
        check(
            &db,
            "comment_create",
            sqlx::query(sql)
                .bind(post)
                .bind(9_i64)
                .bind(Some(1234_i64))
                .bind("x")
                .bind(vec![3_i64]),
            index,
        )
        .await;
    }

    for (name, sql) in [
        ("comment_soft_delete", q!("comment_soft_delete.sql")),
        ("comment_restore", q!("comment_restore.sql")),
    ] {
        let q = if name == "comment_restore" {
            sqlx::query(sql).bind(1234_i64)
        } else {
            sqlx::query(sql).bind(1234_i64).bind(1_i16)
        };
        check(&db, name, q, "floor_comments_pkey").await;
    }

    // 合并点赞通知：水位之后的赞只读索引
    let since: time::OffsetDateTime = sqlx::query_scalar("SELECT now() - interval '10 minutes'")
        .fetch_one(&db)
        .await
        .unwrap();
    let sql = q!("likes_digest.sql");
    check(
        &db,
        "likes_digest",
        sqlx::query(sql).bind(post).bind(Some(since)).bind(3_i32),
        "post_likes_recent",
    )
    .await;

    // 时间轴：100 个采样楼层走 (topic_id, floor) 唯一索引
    let floors: Vec<i32> = (0..100).map(|i| 1 + i * 20).collect();
    let sql = q!("topic_timeline.sql");
    check(
        &db,
        "topic_timeline",
        sqlx::query(sql).bind(2_i64).bind(&floors),
        "posts_topic_id_floor_key",
    )
    .await;

    // 主题列表的库存：交易帖下的公开报价，再按报价哈希数订单
    let listings: Vec<i64> = (1..=50).collect();
    let sql = q!("listing_stock.sql");
    for index in ["offers_listing", "escrow_orders_offer"] {
        check(
            &db,
            "listing_stock",
            sqlx::query(sql).bind(&listings),
            index,
        )
        .await;
    }

    let names: Vec<String> = (1..=5).map(|i| format!("user_{i}")).collect();
    let sql = q!("users_ids_by_names.sql");
    check(
        &db,
        "users_ids_by_names",
        sqlx::query(sql).bind(&names),
        "users_username",
    )
    .await;

    // 冲突检查走唯一约束的索引，计划里只有插入本身
    let sql = q!("report_create.sql");
    let (info, plan) = explain(
        &db,
        sqlx::query(sql)
            .bind(1_i16)
            .bind(5_i64)
            .bind(1_i32)
            .bind(7_i64)
            .bind("")
            .bind(5_i64),
    )
    .await;
    assert!(info.seq_scans.is_empty(), "report_create: {plan:#}");
}

/// 举报队列、楼层隐藏与热门排序（迁移 20261211000001）。
#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn report_queue_and_hot_queries_use_indexes(db: PgPool) {
    seed(&db).await;
    sqlx::raw_sql(
        r"
        -- 主题分布到近 500 天（热门只看近 14 天：约 1300 个），1 楼带点赞计数
        UPDATE topics SET created_at = now() - id * interval '15 minutes', last_post_at = now() - id * interval '14 minutes';
        INSERT INTO post_counters (post_id, likes) SELECT id, id % 13 FROM posts WHERE floor = 1;
        INSERT INTO floor_comments (post_id, author_id, body) SELECT id, author_id, 'c' FROM posts;
        -- 2 万件已结案的举报案件、每板块几十件待处理，各一两条举报
        INSERT INTO report_cases (target_kind, target_id, forum_id, target_author_id, status, action, resolved_by, resolved_at)
        SELECT 1 + g % 2, g, 1 + g % 3, 1 + g % 1000,
               CASE WHEN g % 200 = 0 THEN 0 ELSE 2 END, CASE WHEN g % 200 = 0 THEN NULL ELSE 'delete' END,
               CASE WHEN g % 200 = 0 THEN NULL ELSE 1 END, CASE WHEN g % 200 = 0 THEN NULL ELSE now() END
        FROM generate_series(1, 20000) g;
        INSERT INTO reports (target_kind, target_id, forum_id, reporter_id, case_id)
        SELECT c.target_kind, c.target_id, c.forum_id, 1 + (c.id * 3 + k) % 1000, c.id
        FROM report_cases c, generate_series(1, 2) k;
        INSERT INTO hot_topics (topic_id, forum_id, score) SELECT id, forum_id, 1.0 / id FROM topics WHERE id <= 3000;
        ANALYZE;
        ",
    )
    .execute(&db)
    .await
    .unwrap();
    let boards = vec![1_i32, 2, 3];

    for (name, sql, index) in [
        (
            "report_queue_open",
            q!("report_queue_open.sql"),
            "report_cases_open",
        ),
        (
            "report_queue_resolved",
            q!("report_queue_resolved.sql"),
            "report_cases_resolved",
        ),
    ] {
        check(
            &db,
            name,
            sqlx::query(sql)
                .bind(&boards)
                .bind(i64::MAX)
                .bind(31_i64)
                .bind(7_i64),
            index,
        )
        .await;
    }

    let cases: Vec<i64> = (19_900..19_930).collect();
    let sql = q!("reports_of_cases.sql");
    check(
        &db,
        "reports_of_cases",
        sqlx::query(sql).bind(&cases).bind(5_i64),
        "reports_case",
    )
    .await;

    // 写语句：案件按部分唯一索引 / 主键定位，结案通知按案件取举报人
    let sql = q!("report_case_upsert.sql");
    let (info, plan) = explain(
        &db,
        sqlx::query(sql)
            .bind(1_i16)
            .bind(5_i64)
            .bind(1_i32)
            .bind(7_i64),
    )
    .await;
    assert!(info.seq_scans.is_empty(), "report_case_upsert: {plan:#}");

    let sql = q!("report_case_resolve.sql");
    for index in ["report_cases_pkey", "reports_case"] {
        check(
            &db,
            "report_case_resolve",
            sqlx::query(sql)
                .bind(200_i64)
                .bind(2_i16)
                .bind("hide")
                .bind(1_i64)
                .bind(2_i64)
                .bind(3_i32),
            index,
        )
        .await;
    }

    // 主题移动：只扫待处理的案件（部分索引），按主键回查楼层与评论
    let sql = q!("report_cases_follow_topic.sql");
    let open = ["report_cases_open", "report_cases_open_target"];
    for indexes in [&open[..], &["posts_pkey"]] {
        check_any(
            &db,
            "report_cases_follow_topic",
            sqlx::query(sql).bind(2_i64).bind(3_i32),
            indexes,
        )
        .await;
    }

    // 热门重算：近 14 天的主题走 topics_latest 范围扫描，1 楼与计数按唯一索引 / 主键各取一行
    let sql = q!("hot_refresh.sql");
    for index in [
        "topics_latest",
        "posts_topic_id_floor_key",
        "post_counters_pkey",
    ] {
        check(&db, "hot_refresh", sqlx::query(sql).bind(1000_i64), index).await;
    }

    let sql = q!("hot_board.sql");
    check(
        &db,
        "hot_board",
        sqlx::query(sql)
            .bind(2_i32)
            .bind(f64::INFINITY)
            .bind(i64::MAX)
            .bind(50_i64),
        "hot_topics_forum",
    )
    .await;
    let sql = q!("hot_latest.sql");
    check(
        &db,
        "hot_latest",
        sqlx::query(sql)
            .bind(&boards)
            .bind(0.01_f64)
            .bind(100_i64)
            .bind(50_i64),
        "hot_topics_all",
    )
    .await;
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn search_queries_use_indexes(db: PgPool) {
    seed(&db).await;
    // 第二步回表：Groonga 给出的一页候选按主键取楼层与主题，不扫大表
    let ids: Vec<i64> = (1..=60).map(|i| i * 700).collect();
    let sql = q!("search_posts_rows.sql");
    check(
        &db,
        "search_posts_rows",
        sqlx::query(sql).bind(&ids).bind([1_i32, 2, 3]),
        "posts_pkey",
    )
    .await;
    check(
        &db,
        "search_posts_rows (topics)",
        sqlx::query(sql).bind(&ids).bind([1_i32, 2, 3]),
        "topics_pkey",
    )
    .await;
    // 第一步只调用 pgroonga_command，PostgreSQL 这边不碰任何表
    for (name, sql) in [
        ("search_groonga_posts", q!("search_groonga_posts.sql")),
        ("search_groonga_topics", q!("search_groonga_topics.sql")),
    ] {
        let (info, plan) = explain(
            &db,
            sqlx::query(sql).bind("\"first\"").bind("").bind(20_i64),
        )
        .await;
        assert!(
            info.seq_scans.is_empty() && info.indexes.is_empty(),
            "{name}: {plan:#}"
        );
    }
}
