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
async fn outbox_claims_by_priority_and_marks_sent(pool: PgPool) {
    let low = outbox::enqueue(&pool, "k", &json!({"n": 1}), outbox::PRIORITY_NORMAL)
        .await
        .unwrap();
    let high = outbox::enqueue(&pool, "k", &json!({"n": 2}), outbox::PRIORITY_HIGH)
        .await
        .unwrap();

    let first = outbox::claim(&pool, 1, 60.0).await.unwrap();
    assert_eq!(first[0].id, high, "higher priority first");
    assert_eq!(first[0].attempts, 1);
    // 认领中的消息别的 worker 取不到
    let second = outbox::claim(&pool, 10, 60.0).await.unwrap();
    assert_eq!(second.iter().map(|m| m.id).collect::<Vec<_>>(), vec![low]);

    // 令牌不对（不是这次认领）不生效
    assert!(!outbox::mark_sent(&pool, high, 0).await.unwrap());
    assert!(outbox::mark_sent(&pool, high, 1).await.unwrap());
    assert!(
        !outbox::mark_sent(&pool, high, 1).await.unwrap(),
        "only once"
    );
    let payload: serde_json::Value = sqlx::query_scalar("SELECT payload FROM outbox WHERE id = $1")
        .bind(high)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(payload, json!({}), "payload cleared after send");
}

/// 同一订单的 `order.updated` 一次只认领最早未完成的一条；别的订单与别的消息不受影响。
#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn outbox_orders_messages_per_key(pool: PgPool) {
    let order = |id: i64, state: &str| json!({"id": id, "state": state});
    let a1 = outbox::enqueue(&pool, "order.updated", &order(1, "funded"), 0)
        .await
        .unwrap();
    let a2 = outbox::enqueue(&pool, "order.updated", &order(1, "shipped"), 0)
        .await
        .unwrap();
    let b1 = outbox::enqueue(&pool, "order.updated", &order(2, "funded"), 0)
        .await
        .unwrap();
    let other = outbox::enqueue(&pool, "notify.reply", &json!({"to": 1}), 0)
        .await
        .unwrap();
    let ids = |m: Vec<outbox::Message>| m.into_iter().map(|m| m.id).collect::<Vec<_>>();

    let first = outbox::claim(&pool, 10, 0.0).await.unwrap();
    assert_eq!(ids(first.clone()), vec![a1, b1, other]);
    // a1 在退避中：a2 仍然等着
    assert_eq!(
        outbox::mark_failed(&pool, a1, first[0].attempts, "boom", false)
            .await
            .unwrap(),
        Some(false)
    );
    for m in &first[1..] {
        outbox::mark_sent(&pool, m.id, m.attempts).await.unwrap();
    }
    sqlx::query("UPDATE outbox SET next_attempt_at = now() WHERE id = $1")
        .bind(a1)
        .execute(&pool)
        .await
        .unwrap();
    let again = outbox::claim(&pool, 10, 0.0).await.unwrap();
    assert_eq!(ids(again.clone()), vec![a1]);
    // a1 进了死信之后 a2 不再被挡
    assert_eq!(
        outbox::mark_failed(&pool, a1, again[0].attempts, "bad", true)
            .await
            .unwrap(),
        Some(true)
    );
    assert_eq!(ids(outbox::claim(&pool, 10, 0.0).await.unwrap()), vec![a2]);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn staff_alerts_dedupe_by_event_even_after_sending(pool: PgPool) {
    let first = outbox::staff_alert_deduped(&pool, "sponsor.hot_wallet_low", "低", 3600.0)
        .await
        .unwrap();
    assert!(first);
    // 发送后 payload 被清空，去重仍然生效。
    let id: i64 =
        sqlx::query_scalar("SELECT id FROM outbox WHERE dedupe_key = 'sponsor.hot_wallet_low'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let claimed = outbox::claim(&pool, 10, 60.0).await.unwrap();
    assert!(
        outbox::mark_sent(&pool, id, claimed[0].attempts)
            .await
            .unwrap()
    );
    assert!(
        !outbox::staff_alert_deduped(&pool, "sponsor.hot_wallet_low", "还是低", 3600.0)
            .await
            .unwrap()
    );
    assert!(
        outbox::staff_alert_deduped(&pool, "sponsor.paymaster_deposit_low", "押金低", 3600.0)
            .await
            .unwrap(),
        "other events are independent"
    );
    // 窗口过了再发；不去重的告警总是写入。
    assert!(
        outbox::staff_alert_deduped(&pool, "sponsor.hot_wallet_low", "又低了", 0.0)
            .await
            .unwrap()
    );
    outbox::staff_alert(&pool, "sponsor.hot_wallet_low", "手动")
        .await
        .unwrap();
    let rows: Vec<(String, serde_json::Value)> =
        sqlx::query_as("SELECT kind, payload FROM outbox WHERE sent_at IS NULL ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|(k, _)| k == outbox::KIND_STAFF_ALERT));
    assert_eq!(
        rows[0].1,
        json!({"event": "sponsor.paymaster_deposit_low", "text": "押金低"})
    );
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn outbox_backoff_then_dead_letter_then_requeue(pool: PgPool) {
    let id = outbox::enqueue(&pool, "k", &json!({"secret": 1}), 0)
        .await
        .unwrap();
    let due_in = || async {
        sqlx::query_scalar::<_, f64>(
            "SELECT extract(epoch FROM next_attempt_at - now())::float8 FROM outbox WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    let make_due = || async {
        sqlx::query("UPDATE outbox SET next_attempt_at = now() WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
    };
    for attempt in 1..=outbox::MAX_ATTEMPTS {
        let m = outbox::claim(&pool, 10, 60.0).await.unwrap();
        assert_eq!(m[0].attempts, attempt);
        let dead = outbox::mark_failed(&pool, id, attempt, "boom", false)
            .await
            .unwrap();
        assert_eq!(dead, Some(attempt == outbox::MAX_ATTEMPTS));
        if attempt < 6 {
            // 2^n 秒乘 0.5–1
            let base = f64::from(1 << attempt);
            let d = due_in().await;
            assert!(d > base * 0.5 - 1.0 && d <= base, "attempt {attempt}: {d}");
            assert!(
                outbox::claim(&pool, 10, 60.0).await.unwrap().is_empty(),
                "not due during backoff"
            );
        }
        make_due().await;
    }
    // 死信：不再认领，payload 保留
    assert!(outbox::claim(&pool, 10, 60.0).await.unwrap().is_empty());
    let dead = outbox::dead_letters(&pool, None, None, 10).await.unwrap();
    assert_eq!(dead.len(), 1);
    assert_eq!(
        (dead[0].id, dead[0].attempts, dead[0].last_error.as_deref()),
        (id, outbox::MAX_ATTEMPTS, Some("boom"))
    );
    assert!(
        outbox::dead_letters(&pool, Some("other"), None, 10)
            .await
            .unwrap()
            .is_empty()
    );
    let counts = outbox::counts(&pool).await.unwrap();
    assert_eq!((counts[0].pending, counts[0].dead), (0, 1));

    // 重新投递：从头计数、立即到期、payload 原样
    let handled = outbox::requeue(&pool, &[id, id + 100]).await.unwrap();
    assert_eq!(handled.iter().map(|h| h.id).collect::<Vec<_>>(), vec![id]);
    assert!(
        outbox::requeue(&pool, &[id]).await.unwrap().is_empty(),
        "not dead any more"
    );
    let m = outbox::claim(&pool, 10, 60.0).await.unwrap();
    assert_eq!((m[0].attempts, &m[0].payload), (1, &json!({"secret": 1})));

    // 丢弃：清空 payload，不能再重新投递
    outbox::mark_failed(&pool, id, 1, "bad", true)
        .await
        .unwrap();
    assert_eq!(outbox::discard(&pool, &[id]).await.unwrap().len(), 1);
    assert!(outbox::requeue(&pool, &[id]).await.unwrap().is_empty());
    let payload: serde_json::Value = sqlx::query_scalar("SELECT payload FROM outbox WHERE id = $1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(payload, json!({}));
    assert!(outbox::counts(&pool).await.unwrap().is_empty());
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn outbox_dead_letters_expire(pool: PgPool) {
    let old = outbox::enqueue(&pool, "k", &json!({"to": "a@example.com"}), 0)
        .await
        .unwrap();
    let fresh = outbox::enqueue(&pool, "k", &json!({"to": "b@example.com"}), 0)
        .await
        .unwrap();
    for m in outbox::claim(&pool, 10, 60.0).await.unwrap() {
        outbox::mark_failed(&pool, m.id, m.attempts, "bad", true)
            .await
            .unwrap();
    }
    sqlx::query("UPDATE outbox SET failed_at = now() - interval '15 days' WHERE id = $1")
        .bind(old)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        outbox::discard_expired(&pool, outbox::DEAD_LETTER_RETENTION_SECS)
            .await
            .unwrap(),
        1
    );
    let rows: Vec<(i64, bool, serde_json::Value)> =
        sqlx::query_as("SELECT id, discarded_at IS NOT NULL, payload FROM outbox ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        rows,
        vec![
            (old, true, json!({})),
            (fresh, false, json!({"to": "b@example.com"}))
        ]
    );
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

/// ADR 0013 的角色迁移：超级管理员 → 站长、仲裁员 → 管理员，同一用户的重复角色合并；之后只允许 1、3、4。
#[sqlx::test(migrations = false)]
async fn staff_role_migration_merges_retired_roles(pool: PgPool) {
    const STAFF_MIGRATION: i64 = 20261124000001;
    let (before, after): (Vec<_>, Vec<_>) = forum_db::MIGRATOR
        .iter()
        .partition(|m| m.version < STAFF_MIGRATION);
    for m in before {
        sqlx::raw_sql(m.sql.clone()).execute(&pool).await.unwrap();
    }
    sqlx::raw_sql(
        "INSERT INTO forums (slug, name) VALUES ('f', 'f');
         INSERT INTO users (email) SELECT 'u' || g || '@example.com' FROM generate_series(1, 4) g;
         -- 1：超级管理员；2：站长 + 超级管理员；3：仲裁员 + 管理员；4：仲裁员 + 版主
         INSERT INTO staff_roles (user_id, role, forum_id) VALUES
           (1, 2, NULL), (2, 1, NULL), (2, 2, NULL), (3, 5, NULL), (3, 3, NULL), (4, 5, NULL), (4, 4, 1);",
    )
    .execute(&pool)
    .await
    .unwrap();
    for m in after {
        sqlx::raw_sql(m.sql.clone()).execute(&pool).await.unwrap();
    }
    let rows: Vec<(i64, i16)> =
        sqlx::query_as("SELECT user_id, role FROM staff_roles ORDER BY user_id, role")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(rows, [(1, 1), (2, 1), (3, 3), (4, 3), (4, 4)]);
    for role in [2_i16, 5] {
        assert!(
            sqlx::query("INSERT INTO staff_roles (user_id, role) VALUES (1, $1)")
                .bind(role)
                .execute(&pool)
                .await
                .is_err()
        );
    }
}
