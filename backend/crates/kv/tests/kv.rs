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
