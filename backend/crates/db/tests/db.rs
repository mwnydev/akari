//! 需要 PostgreSQL：`DATABASE_URL` 指向可建库的账号（sqlx::test 为每个用例建临时库并跑迁移）。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

use forum_db::{PgPool, audit, outbox, users};
use serde_json::json;

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn upsert_user_reports_creation(pool: PgPool) {
    let (a, created) = users::upsert_by_email(&pool, "a@example.com")
        .await
        .unwrap();
    assert!(created);
    let (b, created) = users::upsert_by_email(&pool, "a@example.com")
        .await
        .unwrap();
    assert!(!created);
    assert_eq!(a.id, b.id);
    assert_eq!(users::find_by_id(&pool, a.id).await.unwrap(), Some(a));
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn email_must_be_normalized(pool: PgPool) {
    assert!(
        users::upsert_by_email(&pool, "A@Example.com")
            .await
            .is_err()
    );
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn device_first_seen(pool: PgPool) {
    let (u, _) = users::upsert_by_email(&pool, "d@example.com")
        .await
        .unwrap();
    assert!(users::touch_device(&pool, u.id, b"dev1").await.unwrap());
    assert!(!users::touch_device(&pool, u.id, b"dev1").await.unwrap());
    assert!(users::touch_device(&pool, u.id, b"dev2").await.unwrap());
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn outbox_priority_skip_locked_and_sent(pool: PgPool) {
    let low = outbox::enqueue(&pool, "k", &json!({"n": 1}), outbox::PRIORITY_NORMAL)
        .await
        .unwrap();
    let high = outbox::enqueue(&pool, "k", &json!({"n": 2}), outbox::PRIORITY_HIGH)
        .await
        .unwrap();

    let mut tx1 = pool.begin().await.unwrap();
    let first = outbox::lock_due(&mut tx1, 1).await.unwrap();
    assert_eq!(first[0].id, high, "higher priority first");

    // 另一个 worker 看不到已锁定的行
    let mut tx2 = pool.begin().await.unwrap();
    let second = outbox::lock_due(&mut tx2, 10).await.unwrap();
    assert_eq!(second.iter().map(|m| m.id).collect::<Vec<_>>(), vec![low]);
    tx2.rollback().await.unwrap();

    outbox::mark_sent(&mut *tx1, high).await.unwrap();
    tx1.commit().await.unwrap();
    let payload: serde_json::Value = sqlx::query_scalar("SELECT payload FROM outbox WHERE id = $1")
        .bind(high)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(payload, json!({}), "payload cleared after send");
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn outbox_backoff_then_give_up(pool: PgPool) {
    let id = outbox::enqueue(&pool, "k", &json!({"secret": 1}), 0)
        .await
        .unwrap();
    outbox::mark_failed(&pool, id, "boom").await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    assert!(
        outbox::lock_due(&mut tx, 10).await.unwrap().is_empty(),
        "not due during backoff"
    );
    tx.rollback().await.unwrap();

    for _ in 1..outbox::MAX_ATTEMPTS {
        outbox::mark_failed(&pool, id, "boom").await.unwrap();
    }
    let (attempts, failed, payload): (i32, bool, serde_json::Value) =
        sqlx::query_as("SELECT attempts, failed_at IS NOT NULL, payload FROM outbox WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(attempts, outbox::MAX_ATTEMPTS);
    assert!(failed);
    assert_eq!(payload, json!({}));
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn audit_log_is_append_only(pool: PgPool) {
    let id = audit::append(
        &pool,
        1,
        "ban_user",
        Some("user:2"),
        Some("spam"),
        &json!({}),
    )
    .await
    .unwrap();
    for sql in [
        "UPDATE admin_audit_logs SET action = 'x' WHERE id = $1",
        "DELETE FROM admin_audit_logs WHERE id = $1",
    ] {
        let err = sqlx::query(sql).bind(id).execute(&pool).await.unwrap_err();
        assert!(err.to_string().contains("append-only"), "{err}");
    }
    let err = sqlx::query("TRUNCATE admin_audit_logs")
        .execute(&pool)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("append-only"), "{err}");
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn post_search_uses_pgroonga(pool: PgPool) {
    sqlx::query(
        "INSERT INTO posts (topic_id, floor, author_id, html, plain_text)
         VALUES (1, 1, 1, '<p>x</p>', '担保交易 USDT 测试'), (1, 2, 1, '<p>y</p>', '普通回复')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let hits: Vec<i32> = sqlx::query_scalar("SELECT floor FROM posts WHERE plain_text &@~ '担保'")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(hits, vec![1]);
}
