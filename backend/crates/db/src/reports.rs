//! 举报与举报处理队列。同一内容待处理的举报归进同一件案件（`report_cases`），处理一次全部结案并通知举报人。

use sqlx::{PgExecutor, Postgres, Transaction};
use time::OffsetDateTime;

/// `target_kind`。
pub const TARGET_POST: i16 = 1;
pub const TARGET_COMMENT: i16 = 2;

/// `report_cases.status`。
pub const STATUS_OPEN: i16 = 0;
pub const STATUS_DISMISSED: i16 = 1;
pub const STATUS_ACTIONED: i16 = 2;

/// `notify.report_resolved`：结案后给每位举报人的通知（payload `{outcome, topic_id, floor}`）。
pub const NOTIFY_RESOLVED: &str = "notify.report_resolved";

/// 被举报的内容。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    pub kind: i16,
    pub id: i64,
    /// 内容现在所在的板块。
    pub forum_id: i32,
    pub author_id: i64,
}

/// 记一条举报并归入案件。同一人举报过同一内容（含已结案的）时不记，返回 `false`。
pub async fn create(
    tx: &mut Transaction<'_, Postgres>,
    target: Target,
    reporter_id: i64,
    reason: &str,
) -> Result<bool, sqlx::Error> {
    let case_id = sqlx::query_file_scalar!(
        "queries/report_case_upsert.sql",
        target.kind,
        target.id,
        target.forum_id,
        target.author_id
    )
    .fetch_one(&mut **tx)
    .await?;
    let r = sqlx::query_file!(
        "queries/report_create.sql",
        target.kind,
        target.id,
        target.forum_id,
        reporter_id,
        reason,
        case_id
    )
    .execute(&mut **tx)
    .await?;
    Ok(r.rows_affected() == 1)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Case {
    pub id: i64,
    pub target_kind: i16,
    pub target_id: i64,
    pub forum_id: i32,
    pub target_author_id: i64,
    pub reports: i32,
    pub last_reported_at: OffsetDateTime,
    pub created_at: OffsetDateTime,
    pub status: i16,
    pub action: Option<String>,
    pub resolved_by: Option<i64>,
    pub resolved_at: Option<OffsetDateTime>,
}

pub async fn case(db: impl PgExecutor<'_>, id: i64) -> Result<Option<Case>, sqlx::Error> {
    sqlx::query_as!(
        Case,
        "SELECT id, target_kind, target_id, forum_id, target_author_id, reports, last_reported_at, created_at,
                status, action, resolved_by, resolved_at
         FROM report_cases WHERE id = $1",
        id
    )
    .fetch_optional(db)
    .await
}

/// 一页队列（新的在前）：`forum_ids` 内的待处理（`open`）或已结案的案件，不含 `viewer` 自己的内容。
/// `before` 为游标（上一页最后一件的 ID）。
pub async fn queue(
    db: impl PgExecutor<'_>,
    forum_ids: &[i32],
    open: bool,
    before: Option<i64>,
    limit: i64,
    viewer: i64,
) -> Result<Vec<Case>, sqlx::Error> {
    let before = before.unwrap_or(i64::MAX);
    if open {
        sqlx::query_file_as!(
            Case,
            "queries/report_queue_open.sql",
            forum_ids,
            before,
            limit,
            viewer
        )
        .fetch_all(db)
        .await
    } else {
        sqlx::query_file_as!(
            Case,
            "queries/report_queue_resolved.sql",
            forum_ids,
            before,
            limit,
            viewer
        )
        .fetch_all(db)
        .await
    }
}

/// `forum_ids` 内待处理的案件数（不含 `viewer` 自己的内容）。
pub async fn open_count(
    db: impl PgExecutor<'_>,
    forum_ids: &[i32],
    viewer: i64,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!" FROM report_cases
           WHERE status = 0 AND forum_id = ANY($1) AND target_author_id <> $2"#,
        forum_ids,
        viewer
    )
    .fetch_one(db)
    .await
}

/// 各板块待处理的案件数（后台概览），只列有待处理案件的板块。
pub async fn open_by_forum(db: impl PgExecutor<'_>) -> Result<Vec<(i32, i64)>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT forum_id, count(*) AS "n!" FROM report_cases WHERE status = 0
           GROUP BY forum_id ORDER BY forum_id"#
    )
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(|r| (r.forum_id, r.n)).collect())
}

/// 案件里的一条举报。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub case_id: i64,
    pub reporter_id: i64,
    pub reason: String,
    pub created_at: OffsetDateTime,
}

/// 每件案件最近的 `per_case` 条举报，新的在前。
pub async fn latest_reports(
    db: impl PgExecutor<'_>,
    case_ids: &[i64],
    per_case: i64,
) -> Result<Vec<Report>, sqlx::Error> {
    if case_ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_file_as!(Report, "queries/reports_of_cases.sql", case_ids, per_case)
        .fetch_all(db)
        .await
}

/// 结案并通知每位举报人；案件已结案时返回 `false`（什么也不改）。`topic_id` / `floor` 写进通知，方便举报人回看。
pub async fn resolve(
    db: impl PgExecutor<'_>,
    case_id: i64,
    status: i16,
    action: Option<&str>,
    by: i64,
    topic_id: i64,
    floor: i32,
) -> Result<bool, sqlx::Error> {
    sqlx::query_file_scalar!(
        "queries/report_case_resolve.sql",
        case_id,
        status,
        action,
        by,
        topic_id,
        floor
    )
    .fetch_one(db)
    .await
}

/// 撤销结案（站长一键撤销）：重新打开案件；之后同一内容已有新的待处理案件时不改，返回 `false`。
pub async fn reopen(db: impl PgExecutor<'_>, case_id: i64) -> Result<bool, sqlx::Error> {
    let r = sqlx::query!(
        "UPDATE report_cases c SET status = 0, action = NULL, resolved_by = NULL, resolved_at = NULL
         WHERE c.id = $1 AND c.status <> 0
           AND NOT EXISTS (SELECT 1 FROM report_cases o
                           WHERE o.target_kind = c.target_kind AND o.target_id = c.target_id AND o.status = 0)",
        case_id
    )
    .execute(db)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// 主题移到 `forum_id` 后，它的楼层与评论上待处理的案件跟过去（由新板块的版主处理）。
pub async fn follow_topic(
    db: impl PgExecutor<'_>,
    topic_id: i64,
    forum_id: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query_file!("queries/report_cases_follow_topic.sql", topic_id, forum_id)
        .execute(db)
        .await?;
    Ok(())
}

/// 队列里被举报楼层的摘要。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostPreview {
    pub id: i64,
    pub topic_id: i64,
    pub floor: i32,
    pub author_id: i64,
    pub status: i16,
    /// 纯文本前 200 字。
    pub excerpt: String,
    pub title: String,
    pub forum_id: i32,
}

pub async fn post_previews(
    db: impl PgExecutor<'_>,
    ids: &[i64],
) -> Result<Vec<PostPreview>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as!(
        PostPreview,
        r#"SELECT p.id, p.topic_id, p.floor, p.author_id, p.status, left(p.plain_text, 200) AS "excerpt!",
                  t.title, t.forum_id
           FROM posts p JOIN topics t ON t.id = p.topic_id
           WHERE p.id = ANY($1)"#,
        ids
    )
    .fetch_all(db)
    .await
}

/// 队列里被举报评论的摘要。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentPreview {
    pub id: i64,
    pub post_id: i64,
    pub author_id: i64,
    pub status: i16,
    pub excerpt: String,
    pub topic_id: i64,
    pub floor: i32,
    pub title: String,
    pub forum_id: i32,
}

pub async fn comment_previews(
    db: impl PgExecutor<'_>,
    ids: &[i64],
) -> Result<Vec<CommentPreview>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as!(
        CommentPreview,
        r#"SELECT c.id, c.post_id, c.author_id, c.status, left(c.body, 200) AS "excerpt!",
                  p.topic_id, p.floor, t.title, t.forum_id
           FROM floor_comments c JOIN posts p ON p.id = c.post_id JOIN topics t ON t.id = p.topic_id
           WHERE c.id = ANY($1)"#,
        ids
    )
    .fetch_all(db)
    .await
}
