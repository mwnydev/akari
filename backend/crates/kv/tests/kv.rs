//! 需要 Valkey：设置 `VALKEY_URL`，否则跳过。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

use std::time::Duration;

use forum_kv::{SlidingWindow, ValkeySessionStore};
use tower_sessions_core::SessionStore;
use tower_sessions_core::session::{Id, Record};

fn pool() -> Option<forum_kv::Pool> {
    let Ok(url) = std::env::var("VALKEY_URL") else {
        eprintln!("VALKEY_URL not set, skipping valkey-backed test");
        return None;
    };
    Some(forum_kv::pool(&url).unwrap())
}

fn unique() -> String {
    format!("{}", Id::default())
}

#[tokio::test]
async fn sliding_window_limits_and_recovers() {
    let Some(pool) = pool() else { return };
    let rule = SlidingWindow::new("test", 3, Duration::from_millis(300));
    let id = unique();
    for _ in 0..3 {
        assert!(rule.hit(&pool, &id).await.unwrap());
    }
    assert!(!rule.hit(&pool, &id).await.unwrap(), "4th hit rejected");
    // 被拒绝的请求不占名额：窗口过去后立即恢复
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert!(rule.hit(&pool, &id).await.unwrap());
}

#[tokio::test]
async fn ephemeral_take_is_single_use_and_expires() {
    let Some(pool) = pool() else { return };
    let key = format!("test:eph:{}", unique());
    forum_kv::ephemeral::put(&pool, &key, "a", Duration::from_secs(5))
        .await
        .unwrap();
    // 覆盖写入：旧值作废
    forum_kv::ephemeral::put(&pool, &key, "b", Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(
        forum_kv::ephemeral::take(&pool, &key).await.unwrap(),
        Some("b".to_owned())
    );
    assert_eq!(forum_kv::ephemeral::take(&pool, &key).await.unwrap(), None);
    forum_kv::ephemeral::put(&pool, &key, "c", Duration::from_millis(100))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(forum_kv::ephemeral::take(&pool, &key).await.unwrap(), None);
}

#[tokio::test]
async fn session_store_roundtrip_and_expiry() {
    let Some(pool) = pool() else { return };
    let store = ValkeySessionStore::new(pool);
    let mut record = Record {
        id: Id::default(),
        data: [("uid".to_owned(), serde_json::json!(42))]
            .into_iter()
            .collect(),
        expiry_date: time::OffsetDateTime::now_utc() + time::Duration::seconds(2),
    };
    store.create(&mut record).await.unwrap();
    let loaded = store.load(&record.id).await.unwrap().unwrap();
    assert_eq!(loaded.data, record.data);

    store.delete(&record.id).await.unwrap();
    assert!(store.load(&record.id).await.unwrap().is_none());

    // 过期由 Valkey EXAT 负责
    store.save(&record).await.unwrap();
    tokio::time::sleep(Duration::from_millis(3100)).await;
    assert!(store.load(&record.id).await.unwrap().is_none());
}

mod forum_cache {
    use std::time::Duration;

    use forum_kv::ForumCache;
    use forum_kv::forum_cache::BOARD_CAP;

    fn cache() -> Option<ForumCache> {
        let pool = super::pool()?;
        Some(ForumCache::new(pool, format!("t{}:", super::unique())))
    }

    #[tokio::test]
    async fn board_not_built_until_filled() {
        let Some(c) = cache() else { return };
        assert_eq!(c.board_page(1, 0, 49).await.unwrap(), None);
        // 未建好前的写入不丢：重建时合并
        c.board_touch(1, 7, 700).await.unwrap();
        assert_eq!(c.board_page(1, 0, 49).await.unwrap(), None);
        c.board_fill(1, &[(5, 500), (6, 600)]).await.unwrap();
        let page = c.board_page(1, 0, 49).await.unwrap().unwrap();
        assert_eq!(page.total, 3);
        assert_eq!(page.entries, vec![(7, 700), (6, 600), (5, 500)]);
    }

    #[tokio::test]
    async fn board_touch_reorders_and_remove() {
        let Some(c) = cache() else { return };
        c.board_fill(2, &[(1, 100), (2, 200), (3, 300)])
            .await
            .unwrap();
        c.board_touch(2, 1, 1_792_367_748_600_498).await.unwrap(); // 真实量级的微秒时间戳
        c.board_remove(2, 2).await.unwrap();
        let page = c.board_page(2, 0, 1).await.unwrap().unwrap();
        assert_eq!(page.entries, vec![(1, 1_792_367_748_600_498), (3, 300)]);
        assert_eq!(page.total, 2);
        // 空板块也能建好
        c.board_fill(3, &[]).await.unwrap();
        assert_eq!(c.board_page(3, 0, 49).await.unwrap().unwrap().total, 0);
    }

    #[tokio::test]
    async fn board_capped() {
        let Some(c) = cache() else { return };
        let entries: Vec<(i64, i64)> = (1..=(BOARD_CAP as i64 + 5)).map(|i| (i, i)).collect();
        c.board_fill(4, &entries).await.unwrap();
        c.board_touch(4, 99_999, 99_999).await.unwrap();
        let page = c.board_page(4, 0, 0).await.unwrap().unwrap();
        assert_eq!(page.total, BOARD_CAP as u64);
        assert_eq!(page.entries, vec![(99_999, 99_999)]);
    }

    #[tokio::test]
    async fn versioned_cache_invalidation_beats_stale_writeback() {
        let Some(c) = cache() else { return };
        let (v0, val) = c.versioned_get("x").await.unwrap();
        assert_eq!(val, None);
        // 读者拿到版本后，写者失效；读者再用旧版本写回 → 新读者看不到
        c.bump("x").await.unwrap();
        c.versioned_set("x", &v0, "stale", Duration::from_secs(60))
            .await
            .unwrap();
        let (v1, val) = c.versioned_get("x").await.unwrap();
        assert_ne!(v0, v1);
        assert_eq!(val, None);
        c.versioned_set("x", &v1, "fresh", Duration::from_secs(60))
            .await
            .unwrap();
        assert_eq!(
            c.versioned_get("x").await.unwrap().1.as_deref(),
            Some("fresh")
        );
    }

    #[tokio::test]
    async fn views_take_and_restore() {
        let Some(c) = cache() else { return };
        for _ in 0..3 {
            c.add_view(10).await.unwrap();
        }
        c.add_view(11).await.unwrap();
        let mut taken = c.take_views().await.unwrap();
        taken.sort_unstable();
        assert_eq!(taken, vec![(10, 3), (11, 1)]);
        assert!(c.take_views().await.unwrap().is_empty());
        c.restore_views(&taken).await.unwrap();
        c.add_view(10).await.unwrap();
        let mut again = c.take_views().await.unwrap();
        again.sort_unstable();
        assert_eq!(again, vec![(10, 4), (11, 1)]);
    }
}
