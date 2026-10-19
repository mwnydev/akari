//! 楼层。删除只改状态，楼层号保留。

use serde_json::Value;
use sqlx::{PgExecutor, Postgres, Transaction};
use time::OffsetDateTime;

pub const STATUS_NORMAL: i16 = 0;
pub const STATUS_DELETED_BY_AUTHOR: i16 = 1;
pub const STATUS_DELETED_BY_STAFF: i16 = 2;
/// 每页楼层数（plan 第 9 节）。
pub const PAGE_SIZE: i32 = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Post {
    pub id: i64,
    pub topic_id: i64,
    pub floor: i32,
    pub author_id: i64,
    pub html: String,
    pub status: i16,
    pub created_at: OffsetDateTime,
    pub edited_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Copy)]
pub struct Replied {
    pub post_id: i64,
    pub floor: i32,
    pub forum_id: i32,
    pub flags: i16,
    pub last_post_at: OffsetDateTime,
}

#[derive(Debug, Clone, Copy)]
pub struct NewReply<'a> {
    pub topic_id: i64,
    pub author_id: i64,
    pub html: &'a str,
    pub plain_text: &'a str,
    pub source: &'a Value,
    pub mentions: &'a [i64],
    /// 管理人员可回复锁定的主题。
    pub allow_locked: bool,
}

/// 回帖。主题不存在、已隐藏或已锁定（且无权回复锁定帖）时返回 `None`。
pub async fn reply(
    db: impl PgExecutor<'_>,
    r: NewReply<'_>,
) -> Result<Option<Replied>, sqlx::Error> {
    let row = sqlx::query_file!(
        "queries/post_reply.sql",
        r.topic_id,
        r.author_id,
        r.html,
        r.plain_text,
        r.source,
        r.mentions,
        r.allow_locked
    )
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| Replied {
        post_id: r.post_id,
        floor: r.floor,
        forum_id: r.forum_id,
        flags: r.flags,
        last_post_at: r.last_post_at,
    }))
}

/// 第 `page` 页（从 1 开始）。
pub async fn page(
    db: impl PgExecutor<'_>,
    topic_id: i64,
    page: i32,
) -> Result<Vec<Post>, sqlx::Error> {
    let from = (page - 1).saturating_mul(PAGE_SIZE).saturating_add(1);
    let to = page.saturating_mul(PAGE_SIZE);
    sqlx::query_file_as!(Post, "queries/posts_page.sql", topic_id, from, to)
        .fetch_all(db)
        .await
}

pub async fn by_id(db: impl PgExecutor<'_>, id: i64) -> Result<Option<Post>, sqlx::Error> {
    sqlx::query_file_as!(Post, "queries/posts_by_id.sql", id)
        .fetch_optional(db)
        .await
}

#[derive(Debug, Clone, Copy)]
pub struct Edit<'a> {
    pub post_id: i64,
    pub editor_id: i64,
    pub html: &'a str,
    pub plain_text: &'a str,
    pub source: &'a Value,
    /// 1 楼改标题时的旧标题，存入编辑历史。
    pub old_title: Option<&'a str>,
}

/// 编辑：先把当前版本存进 `post_edits`，再更新。必须在事务内（与改标题、审计日志一起提交）。
/// 返回保存旧版本的那条编辑历史 ID；楼层已删除时不更新，返回 `None`。
pub async fn edit(
    tx: &mut Transaction<'_, Postgres>,
    e: Edit<'_>,
) -> Result<Option<i64>, sqlx::Error> {
    let Some(saved) = sqlx::query_file!(
        "queries/post_edit_save.sql",
        e.post_id,
        e.editor_id,
        e.old_title
    )
    .fetch_optional(&mut **tx)
    .await?
    else {
        return Ok(None);
    };
    sqlx::query_file!(
        "queries/post_edit_update.sql",
        e.post_id,
        e.html,
        e.plain_text
    )
    .execute(&mut **tx)
    .await?;
    sqlx::query_file!("queries/post_source_update.sql", e.post_id, e.source)
        .execute(&mut **tx)
        .await?;
    Ok(Some(saved.id))
}

/// 一条编辑历史保存的旧版本（撤销管理人员的编辑时恢复）。
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub post_id: i64,
    pub title: Option<String>,
    pub html: String,
    pub source: Value,
    /// 这条之后该楼层还有没有别的编辑。
    pub superseded: bool,
}

pub async fn snapshot(
    db: impl PgExecutor<'_>,
    edit_id: i64,
) -> Result<Option<Snapshot>, sqlx::Error> {
    sqlx::query_as!(
        Snapshot,
        r#"SELECT e.post_id, e.title, e.html, e.source,
                  EXISTS (SELECT 1 FROM post_edits n WHERE n.post_id = e.post_id AND n.id > e.id) AS "superseded!"
           FROM post_edits e WHERE e.id = $1"#,
        edit_id
    )
    .fetch_optional(db)
    .await
}

/// 编辑器原稿（作者重新编辑时加载）。
pub async fn source(db: impl PgExecutor<'_>, post_id: i64) -> Result<Option<Value>, sqlx::Error> {
    let row = sqlx::query_file!("queries/post_source.sql", post_id)
        .fetch_optional(db)
        .await?;
    Ok(row.map(|r| r.source))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditRecord {
    pub id: i64,
    pub editor_id: i64,
    pub title: Option<String>,
    pub html: String,
    pub edited_at: OffsetDateTime,
}

/// 编辑历史，新的在前；`before` 为游标（上一页最后一条的 id）。
pub async fn history(
    db: impl PgExecutor<'_>,
    post_id: i64,
    before: Option<i64>,
    limit: i64,
) -> Result<Vec<EditRecord>, sqlx::Error> {
    sqlx::query_file_as!(
        EditRecord,
        "queries/post_history.sql",
        post_id,
        before.unwrap_or(i64::MAX),
        limit
    )
    .fetch_all(db)
    .await
}

/// 恢复被管理人员删除的楼层。返回是否改动。
pub async fn restore(db: impl PgExecutor<'_>, id: i64) -> Result<bool, sqlx::Error> {
    let r = sqlx::query!(
        "UPDATE posts SET status = $2 WHERE id = $1 AND status = $3",
        id,
        STATUS_NORMAL,
        STATUS_DELETED_BY_STAFF
    )
    .execute(db)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// 软删除；已删除的不再改。返回是否改动。
pub async fn soft_delete(
    db: impl PgExecutor<'_>,
    id: i64,
    status: i16,
) -> Result<bool, sqlx::Error> {
    let r = sqlx::query_file!("queries/post_soft_delete.sql", id, status)
        .execute(db)
        .await?;
    Ok(r.rows_affected() == 1)
}
