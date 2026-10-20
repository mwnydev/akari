//! 发帖经验、每日信誉校对、图片转换。需要 PostgreSQL（`DATABASE_URL`）；图片转换另需 `MEDIA_S3_*`，否则跳过。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

use std::sync::Arc;

use forum_db::PgPool;
use forum_kv::ForumCache;
use forum_media::{RustProcessor, Storage, StorageConfig};
use forum_worker::images::{ImageWorker, Outcome};

const FIXTURE: &[u8] = include_bytes!("../../../crates/media/tests/fixtures/gps-orientation6.jpg");

/// 名片失效只是尽力而为：没有 Valkey 时连接失败只记日志。
fn cache() -> ForumCache {
    let url = std::env::var("VALKEY_URL").unwrap_or_else(|_| "redis://127.0.0.1:1".into());
    ForumCache::new(forum_kv::pool(&url).unwrap(), "test:")
}

async fn exec(db: &PgPool, sql: &'static str) {
    sqlx::raw_sql(sql).execute(db).await.unwrap();
}

async fn exp(db: &PgPool, user: i64) -> i64 {
    sqlx::query_scalar("SELECT coalesce((SELECT exp FROM user_levels WHERE user_id = $1), 0)")
        .bind(user)
        .fetch_one(db)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn post_exp_after_24h_with_rules(db: PgPool) {
    exec(
        &db,
        r"
        INSERT INTO users (email) VALUES ('a@example.com'), ('b@example.com');
        INSERT INTO forums (slug, name) VALUES ('f', 'f');
        INSERT INTO topics (forum_id, author_id, title, last_post_at) VALUES (1, 1, 't', now()), (1, 2, 'hidden', now());
        UPDATE topics SET flags = 8 WHERE title = 'hidden';
        INSERT INTO posts (topic_id, floor, author_id, html, plain_text, status, created_at) VALUES
          (1, 1, 1, '', '主题正文', 0, now() - interval '25 hours'),
          (1, 2, 2, '', '你好 世界', 0, now() - interval '25 hours'),
          (1, 3, 1, '', '自己回自己', 0, now() - interval '25 hours'),
          (1, 4, 2, '', '你好   世界', 0, now() - interval '25 hours'),
          (1, 5, 2, '', '😀👍', 0, now() - interval '25 hours'),
          (1, 6, 2, '', '被删了', 1, now() - interval '25 hours'),
          (2, 1, 2, '', '隐藏主题', 0, now() - interval '25 hours'),
          (1, 7, 2, '', '刚发的', 0, now() - interval '1 hour');
        ",
    )
    .await;
    let c = cache();
    assert_eq!(forum_worker::exp::award_posts(&db, &c).await.unwrap(), 7);
    assert_eq!(
        exp(&db, 1).await,
        2,
        "topic +2; own-topic reply not counted"
    );
    assert_eq!(
        exp(&db, 2).await,
        1,
        "one reply; duplicate, emoji-only, deleted, hidden skipped"
    );
    // 已处理过的不再处理；满 24 小时的新帖下次计入
    assert_eq!(forum_worker::exp::award_posts(&db, &c).await.unwrap(), 0);
    exec(
        &db,
        "UPDATE posts SET created_at = now() - interval '25 hours' WHERE floor = 7",
    )
    .await;
    assert_eq!(forum_worker::exp::award_posts(&db, &c).await.unwrap(), 1);
    assert_eq!(exp(&db, 2).await, 2);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn daily_audit_repairs_stats(db: PgPool) {
    exec(
        &db,
        r"
        INSERT INTO users (email) VALUES ('a@example.com'), ('b@example.com'), ('c@example.com');
        INSERT INTO chains (chain_id, name, escrow_address, config_address, token_address, token_symbol,
          token_decimals, start_block, safe_depth, finalized_depth)
        VALUES (1, 't', '\x4444444444444444444444444444444444444444', '\xc0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0',
          '\x3333333333333333333333333333333333333333', 'USDT', 6, 0, 1, 1);
        INSERT INTO escrow_orders (chain_id, contract, chain_order_id, buyer, seller, buyer_id, seller_id, token,
          amount, instant, offer_hash, listing_hash, state, level, created_block, updated_block)
        VALUES (1, '\x4444444444444444444444444444444444444444', 1, '\xb1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1',
          '\xc1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1', 1, 2, '\x3333333333333333333333333333333333333333',
          20000000, false, sha256('o'), sha256('l'), 3, 'finalized', 1, 1);
        -- indexer 漏掉了卖家，买家的计数被改坏
        INSERT INTO user_trade_stats (user_id, success) VALUES (1, 5);
        ",
    )
    .await;
    let c = cache();
    assert_eq!(
        forum_worker::stats::audit_trade_stats(&db, &c)
            .await
            .unwrap(),
        2
    );
    let rows: Vec<(i64, i32)> =
        sqlx::query_as("SELECT user_id, success FROM user_trade_stats ORDER BY user_id")
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(rows, vec![(1, 1), (2, 1)]);
    assert_eq!(
        forum_worker::stats::audit_trade_stats(&db, &c)
            .await
            .unwrap(),
        0
    );
}

async fn storage() -> Option<Arc<Storage>> {
    let cfg = forum_testkit::require_s3(StorageConfig::from_env().unwrap())?;
    let s = Storage::new(&cfg).unwrap();
    s.ensure_bucket().await.unwrap();
    Some(Arc::new(s))
}

/// 模拟浏览器直传后确认：上传原图到临时目录，插入排队中的上传记录。
async fn queued(db: &PgPool, s: &Storage, body: &[u8]) -> (i64, String) {
    let key = forum_media::tmp_key(1);
    let signed = s.presign_upload(&key, "image/jpeg", body.len() as u64);
    let r = reqwest::Client::new()
        .put(&signed.url)
        .header("content-type", "image/jpeg")
        .body(body.to_vec())
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{}", r.status());
    let id = sqlx::query_scalar(
        "INSERT INTO uploads (user_id, status, content_type, size, tmp_key) VALUES (1, 'queued', 'image/jpeg', $1, $2)
         RETURNING id",
    )
    .bind(i32::try_from(body.len()).unwrap())
    .bind(&key)
    .fetch_one(db)
    .await
    .unwrap();
    (id, key)
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn image_worker_strips_exif_and_stores_by_hash(db: PgPool) {
    let Some(storage) = storage().await else {
        return;
    };
    exec(&db, "INSERT INTO users (email) VALUES ('a@example.com')").await;
    let w = ImageWorker {
        db: db.clone(),
        storage: Arc::clone(&storage),
        processor: Arc::new(RustProcessor),
    };
    let (id, tmp) = queued(&db, &storage, FIXTURE).await;
    assert_eq!(w.run_once().await.unwrap(), Some((id, Outcome::Done)));
    assert_eq!(w.run_once().await.unwrap(), None, "queue empty");

    let u = forum_db::uploads::by_id(&db, id).await.unwrap().unwrap();
    assert_eq!(u.status, forum_db::uploads::UploadStatus::Done);
    assert_eq!(
        (u.width, u.height),
        (Some(48), Some(64)),
        "EXIF orientation applied"
    );
    let variants = u.variants.unwrap();
    let url = variants[0]["url"].as_str().unwrap();
    assert!(url.contains("/i/") && url.ends_with(".webp"), "{url}");
    let webp = reqwest::get(url).await.unwrap().bytes().await.unwrap();
    assert_eq!(&webp[8..12], b"WEBP");
    assert!(!webp.windows(4).any(|w| w == b"EXIF"), "EXIF stripped");
    assert!(!webp.windows(3).any(|w| w == b"GPS"));
    assert!(
        url.ends_with(&forum_media::image_key(&webp)),
        "content-hash name"
    );
    assert_eq!(
        storage.get(&tmp, 1 << 20).await.unwrap(),
        None,
        "original deleted"
    );

    // 声称是 JPEG 的坏文件：失败并删除原图；原图不存在：失败
    let (bad, bad_tmp) = queued(&db, &storage, b"\xFF\xD8\xFF\xE0 broken").await;
    assert_eq!(
        w.run_once().await.unwrap(),
        Some((bad, Outcome::Failed("corrupt")))
    );
    assert_eq!(storage.get(&bad_tmp, 1 << 20).await.unwrap(), None);
    let (gone, gone_tmp) = queued(&db, &storage, FIXTURE).await;
    storage.delete(&gone_tmp).await.unwrap();
    assert_eq!(
        w.run_once().await.unwrap(),
        Some((gone, Outcome::Failed("missing")))
    );
}
