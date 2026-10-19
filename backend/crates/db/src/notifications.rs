//! 站内通知（plan 第 4、8 节）：按月分区、保留 180 天；已读是每人一个水位（不更新通知行）。

pub use forum_core::chat::Notification;
use serde_json::Value;
use sqlx::PgExecutor;

/// 每页条数。
pub const PAGE: i64 = 20;
/// 未读数最多数到这里（界面显示 99+）。
pub const UNREAD_CAP: i64 = 100;

pub const KIND_REPLY: &str = "reply";
pub const KIND_MENTION: &str = "mention";
pub const KIND_MESSAGE: &str = "message";
pub const KIND_ORDER: &str = "order";
pub const KIND_OFFER: &str = "offer";
pub const KIND_SYSTEM: &str = "system";

#[derive(Debug, Clone, PartialEq)]
pub struct NewNotification {
    pub user_id: i64,
    pub kind: &'static str,
    pub actor_id: Option<i64>,
    pub payload: Value,
}

/// 批量写入，返回 `(收件人, 通知)`。
pub async fn insert(
    db: impl PgExecutor<'_>,
    items: &[NewNotification],
) -> Result<Vec<(i64, Notification)>, sqlx::Error> {
    if items.is_empty() {
        return Ok(Vec::new());
    }
    let users: Vec<i64> = items.iter().map(|n| n.user_id).collect();
    let kinds: Vec<&str> = items.iter().map(|n| n.kind).collect();
    let actors: Vec<Option<i64>> = items.iter().map(|n| n.actor_id).collect();
    let payloads: Vec<Value> = items.iter().map(|n| n.payload.clone()).collect();
    let rows = sqlx::query!(
        r#"INSERT INTO notifications (user_id, kind, actor_id, payload)
           SELECT * FROM UNNEST($1::bigint[], $2::text[], $3::bigint[], $4::jsonb[])
           RETURNING id, user_id, kind, actor_id, payload, created_at"#,
        &users,
        &kinds as &[&str],
        &actors as &[Option<i64>],
        &payloads
    )
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            (
                r.user_id,
                Notification {
                    id: r.id,
                    kind: r.kind,
                    actor_id: r.actor_id,
                    payload: r.payload,
                    read: false,
                    created_at: r.created_at,
                },
            )
        })
        .collect())
}

/// 已读水位（没读过任何通知为 0）。
pub async fn read_mark(db: impl PgExecutor<'_>, user_id: i64) -> Result<i64, sqlx::Error> {
    Ok(sqlx::query_scalar!(
        "SELECT last_read_id FROM notification_reads WHERE user_id = $1",
        user_id
    )
    .fetch_optional(db)
    .await?
    .unwrap_or(0))
}

/// 游标之前的一页（新的在前）；`read_mark` 为 [`read_mark`] 的结果。
pub async fn before(
    db: impl PgExecutor<'_>,
    user_id: i64,
    before: i64,
    limit: i64,
    read_mark: i64,
) -> Result<Vec<Notification>, sqlx::Error> {
    sqlx::query_file_as!(
        Notification,
        "queries/notifications_before.sql",
        user_id,
        before,
        limit,
        read_mark
    )
    .fetch_all(db)
    .await
}

/// 游标之后的一页（按时间正序，重连补拉）。
pub async fn after(
    db: impl PgExecutor<'_>,
    user_id: i64,
    after: i64,
    limit: i64,
    read_mark: i64,
) -> Result<Vec<Notification>, sqlx::Error> {
    sqlx::query_file_as!(
        Notification,
        "queries/notifications_after.sql",
        user_id,
        after,
        limit,
        read_mark
    )
    .fetch_all(db)
    .await
}

/// 未读数（最多 [`UNREAD_CAP`]）。
pub async fn unread(
    db: impl PgExecutor<'_>,
    user_id: i64,
    read_mark: i64,
) -> Result<i64, sqlx::Error> {
    sqlx::query_file_scalar!(
        "queries/notifications_unread.sql",
        user_id,
        read_mark,
        UNREAD_CAP
    )
    .fetch_one(db)
    .await
}

/// 把 `up_to_id` 及之前的通知标为已读（不超过自己最新一条，不后退）。返回新的水位。
pub async fn mark_read(
    db: impl PgExecutor<'_>,
    user_id: i64,
    up_to_id: i64,
) -> Result<i64, sqlx::Error> {
    let row = sqlx::query!(
        r#"WITH head AS (
             SELECT least($2, coalesce(max(id), 0)) AS id FROM notifications
             WHERE user_id = $1 AND created_at > now() - interval '180 days'
           ), up AS (
             INSERT INTO notification_reads AS r (user_id, last_read_id)
             SELECT $1, head.id FROM head
             ON CONFLICT (user_id) DO UPDATE SET last_read_id = EXCLUDED.last_read_id, updated_at = now()
               WHERE r.last_read_id < EXCLUDED.last_read_id
             RETURNING last_read_id
           )
           SELECT coalesce((SELECT last_read_id FROM up),
                           (SELECT last_read_id FROM notification_reads WHERE user_id = $1), 0) AS "mark!""#,
        user_id,
        up_to_id
    )
    .fetch_one(db)
    .await?;
    Ok(row.mark)
}

/// 分区维护（worker 每天调用）：建好本月与之后两个月的分区，删掉整月早于 180 天的分区。
/// 返回 `(新建, 删除)` 的分区名。
pub async fn maintain_partitions(
    db: impl PgExecutor<'_>,
) -> Result<(Vec<String>, Vec<String>), sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT created AS "created!", dropped AS "dropped!" FROM notifications_maintain()"#
    )
    .fetch_one(db)
    .await?;
    Ok((row.created, row.dropped))
}
