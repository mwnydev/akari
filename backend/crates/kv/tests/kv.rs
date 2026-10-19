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
async fn ephemeral_get_keeps_the_value_until_expiry() {
    let Some(pool) = pool() else { return };
    let key = format!("test:eph:{}", unique());
    assert_eq!(forum_kv::ephemeral::get(&pool, &key).await.unwrap(), None);
    forum_kv::ephemeral::put(&pool, &key, "a", Duration::from_millis(200))
        .await
        .unwrap();
    for _ in 0..2 {
        assert_eq!(
            forum_kv::ephemeral::get(&pool, &key).await.unwrap(),
            Some("a".to_owned())
        );
    }
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(forum_kv::ephemeral::get(&pool, &key).await.unwrap(), None);
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

    #[tokio::test]
    async fn latest_follows_board_touch_and_remove() {
        let Some(c) = cache() else { return };
        assert_eq!(c.latest_page(None, 10).await.unwrap(), None);
        // 两个板块的发帖都进同一个最新有序集合；未建好前的写入在重建时合并
        c.board_touch(1, 10, 1000).await.unwrap();
        c.latest_fill(&[(5, 500)]).await.unwrap();
        c.board_touch(2, 20, 2000).await.unwrap();
        c.board_touch(1, 10, 3000).await.unwrap(); // 新回复：分数更新
        let page = c.latest_page(None, 10).await.unwrap().unwrap();
        assert_eq!(page.entries, vec![(10, 3000), (20, 2000), (5, 500)]);
        assert_eq!(page.total, 3);
        // 分数上限（含）之下的一页
        let page = c.latest_page(Some(2000), 1).await.unwrap().unwrap();
        assert_eq!(page.entries, vec![(20, 2000)]);
        // 隐藏 / 置顶：从板块与最新都移除
        c.board_remove(2, 20).await.unwrap();
        let page = c.latest_page(None, 10).await.unwrap().unwrap();
        assert_eq!(page.entries, vec![(10, 3000), (5, 500)]);
        assert_eq!(
            c.board_page(2, 0, 9).await.unwrap(),
            None,
            "board 2 not built"
        );
    }

    #[tokio::test]
    async fn cards_roundtrip_and_invalidation_blocks_stale_fill() {
        let Some(c) = cache() else { return };
        assert_eq!(c.cards_get(&[1, 2]).await.unwrap(), vec![None, None]);
        c.cards_put(&[(1, vec![("u", "alice".to_owned()), ("l", "2".to_owned())])])
            .await
            .unwrap();
        let got = c.cards_get(&[1, 2]).await.unwrap();
        assert_eq!(got[0].as_ref().unwrap()["u"], "alice");
        assert_eq!(got[1], None);
        // 失效后几秒内，拿着旧数据的回源写回被拒绝
        c.cards_invalidate(&[1]).await.unwrap();
        c.cards_put(&[(1, vec![("u", "stale".to_owned())])])
            .await
            .unwrap();
        assert_eq!(c.cards_get(&[1]).await.unwrap(), vec![None]);
    }
}

/// 两个部署（不同 `KV_PREFIX`）共用一个 Valkey：各自只收到自己的推送。
#[tokio::test]
async fn realtime_channels_are_isolated_by_namespace() {
    use forum_kv::Realtime;
    use forum_kv::realtime::{Envelope, Route};

    let Some(pool) = pool() else { return };
    let url = std::env::var("VALKEY_URL").unwrap();
    let ns = |tag: &str| forum_kv::namespace(Some(&format!("{tag}{}", unique()))).unwrap();
    let (mine, other) = (
        Realtime::new(pool.clone(), ns("a")),
        Realtime::new(pool, ns("b")),
    );
    assert_ne!(mine.channel(), other.channel());

    let (ready_tx, ready) = tokio::sync::oneshot::channel();
    let (got_tx, mut got) = tokio::sync::mpsc::unbounded_channel();
    let channel = mine.channel();
    tokio::spawn(async move {
        forum_kv::realtime::listen(
            &url,
            &channel,
            || {
                let _ = ready_tx.send(());
            },
            |e| {
                let _ = got_tx.send(e);
            },
        )
        .await
    });
    ready.await.unwrap();

    let event = |n: i64| Envelope::new(Route::Users { ids: vec![n] }, &n).unwrap();
    other.publish(&event(1)).await.unwrap();
    mine.publish(&event(2)).await.unwrap();
    // PUBLISH 返回前消息已进入订阅者的发送队列，所以若另一个前缀的那条串了过来，它会先到
    let first = tokio::time::timeout(Duration::from_secs(5), got.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first, event(2));
}

#[tokio::test]
async fn guest_cache_versions_single_flight_and_stale_while_revalidate() {
    use forum_kv::GuestCache;
    use forum_kv::guest_cache::{GuestTtl, Lookup};
    let Some(pool) = pool() else { return };
    let cache = GuestCache::new(pool, format!("test{}:", unique()));
    let versions = [forum_kv::guest_cache::topic(1)];
    let ttl = GuestTtl {
        fresh: Duration::from_millis(200),
        stale: Duration::from_secs(5),
    };
    // 冷启动：第一个拿到回源锁，其余等待
    let Lookup::Build(ticket) = cache.lookup("e", &versions).await.unwrap() else {
        panic!("first lookup should build")
    };
    assert!(matches!(
        cache.lookup("e", &versions).await.unwrap(),
        Lookup::Wait
    ));
    cache.store("e", ticket, "v1", ttl).await.unwrap();
    assert!(matches!(cache.lookup("e", &versions).await.unwrap(), Lookup::Hit(b) if b == "v1"));

    // 过了新鲜期、版本没变：一个请求重建，其余先拿旧值
    tokio::time::sleep(Duration::from_millis(250)).await;
    let Lookup::Build(ticket) = cache.lookup("e", &versions).await.unwrap() else {
        panic!("expired entry should be rebuilt by one request")
    };
    assert!(matches!(cache.lookup("e", &versions).await.unwrap(), Lookup::Stale(b) if b == "v1"));
    cache.store("e", ticket, "v2", ttl).await.unwrap();
    assert!(matches!(cache.lookup("e", &versions).await.unwrap(), Lookup::Hit(b) if b == "v2"));

    // 版本号加一：旧值不再返回（不当作过期内容），回源期间其余请求等待
    cache.bump(&versions).await.unwrap();
    let Lookup::Build(ticket) = cache.lookup("e", &versions).await.unwrap() else {
        panic!("bumped entry should be rebuilt")
    };
    assert!(matches!(
        cache.lookup("e", &versions).await.unwrap(),
        Lookup::Wait
    ));
    // 回源期间又有写入：带旧版本标签写回的值不算命中
    cache.bump(&versions).await.unwrap();
    cache.store("e", ticket, "v3-outdated", ttl).await.unwrap();
    let Lookup::Build(ticket) = cache.lookup("e", &versions).await.unwrap() else {
        panic!("outdated store must not hit")
    };
    // 内容已不存在：放锁并删除条目，之后不再有旧值可返回
    cache.release("e", ticket, true).await.unwrap();
    let Lookup::Build(ticket) = cache.lookup("e", &versions).await.unwrap() else {
        panic!("released lock should be available")
    };
    cache.release("e", ticket, false).await.unwrap();
    // 依赖别的版本号的条目不受影响
    let other = [forum_kv::guest_cache::topic(2)];
    let Lookup::Build(t) = cache.lookup("o", &other).await.unwrap() else {
        panic!()
    };
    cache.store("o", t, "o1", ttl).await.unwrap();
    cache.bump(&versions).await.unwrap();
    assert!(matches!(cache.lookup("o", &other).await.unwrap(), Lookup::Hit(b) if b == "o1"));
}
