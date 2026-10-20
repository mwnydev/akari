//! 楼层点赞、楼层评论与计数（设计稿 v2）。点赞、评论与计数各自一条语句写完，在自动提交下执行。

pub use forum_core::floor::FloorComment;
use sqlx::{PgExecutor, Postgres, Transaction};
use time::OffsetDateTime;

pub const STATUS_NORMAL: i16 = 0;
pub const STATUS_DELETED_BY_AUTHOR: i16 = 1;
pub const STATUS_DELETED_BY_STAFF: i16 = 2;

/// `outbox.kind`：合并的点赞通知（payload `{post_id}`，延迟投递）。
pub const NOTIFY_LIKES: &str = "notify.likes";

/// 点赞 / 取消赞的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Liked {
    /// 点赞时楼层存在且未删除（取消赞不看楼层状态，总为真）。
    pub found: bool,
    /// 这次确实新增 / 删掉了一个赞（重复操作为假）。
    pub changed: bool,
    pub likes: i32,
}

/// 点赞（幂等）。`notify` 为真（不是给自己的楼层点赞）时，在 `window_secs` 秒后投递一条合并的点赞通知；
/// 窗口内的赞并进同一条。
pub async fn like(
    db: impl PgExecutor<'_>,
    post_id: i64,
    user_id: i64,
    notify: bool,
    window_secs: f64,
) -> Result<Liked, sqlx::Error> {
    let r = sqlx::query_file!(
        "queries/post_like.sql",
        post_id,
        user_id,
        notify,
        window_secs
    )
    .fetch_one(db)
    .await?;
    Ok(Liked {
        found: r.found,
        changed: r.changed,
        likes: r.likes,
    })
}

/// 取消赞（幂等）。
pub async fn unlike(
    db: impl PgExecutor<'_>,
    post_id: i64,
    user_id: i64,
) -> Result<Liked, sqlx::Error> {
    let r = sqlx::query_file!("queries/post_unlike.sql", post_id, user_id)
        .fetch_one(db)
        .await?;
    Ok(Liked {
        found: true,
        changed: r.changed,
        likes: r.likes,
    })
}

/// 一楼的点赞与评论概况。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Social {
    pub post_id: i64,
    pub likes: i32,
    pub comments: i32,
    /// 最近点赞的几个人，新的在前。
    pub likers: Vec<i64>,
    /// 访问者是否已赞。
    pub liked: bool,
}

/// 一页楼层的点赞与评论概况（顺序与 `post_ids` 一致）。`viewer` 为空（游客）时 `liked` 都为假。
pub async fn social(
    db: impl PgExecutor<'_>,
    post_ids: &[i64],
    viewer: Option<i64>,
    likers: i64,
) -> Result<Vec<Social>, sqlx::Error> {
    if post_ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_file_as!(
        Social,
        "queries/posts_social.sql",
        post_ids,
        viewer.unwrap_or(0),
        likers
    )
    .fetch_all(db)
    .await
}

/// 一页楼层各自最早的 `n` 条评论，按楼层、时间排序。
pub async fn preview(
    db: impl PgExecutor<'_>,
    post_ids: &[i64],
    n: i64,
) -> Result<Vec<FloorComment>, sqlx::Error> {
    if post_ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_file_as!(FloorComment, "queries/comments_preview.sql", post_ids, n)
        .fetch_all(db)
        .await
}

/// 一楼在游标 `after`（不含）之后的评论，按时间正序。
pub async fn comments_after(
    db: impl PgExecutor<'_>,
    post_id: i64,
    after: i64,
    limit: i64,
) -> Result<Vec<FloorComment>, sqlx::Error> {
    sqlx::query_file_as!(
        FloorComment,
        "queries/comments_after.sql",
        post_id,
        after,
        limit
    )
    .fetch_all(db)
    .await
}

/// 一条评论（含已删除的）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentRow {
    pub id: i64,
    pub post_id: i64,
    pub author_id: i64,
    pub reply_to_id: Option<i64>,
    pub body: String,
    pub status: i16,
    pub created_at: OffsetDateTime,
}

impl CommentRow {
    pub fn view(&self) -> FloorComment {
        FloorComment {
            id: self.id,
            post_id: self.post_id,
            author_id: self.author_id,
            reply_to_id: self.reply_to_id,
            body: self.body.clone(),
            created_at: self.created_at,
        }
    }
}

pub async fn comment(db: impl PgExecutor<'_>, id: i64) -> Result<Option<CommentRow>, sqlx::Error> {
    sqlx::query_file_as!(CommentRow, "queries/comment_by_id.sql", id)
        .fetch_optional(db)
        .await
}

#[derive(Debug, Clone, Copy)]
pub struct NewComment<'a> {
    pub post_id: i64,
    pub author_id: i64,
    /// 回复的那条评论（必须在同一楼且未删除）。
    pub reply_to: Option<i64>,
    /// 已经过 `forum_content::clean_comment`。
    pub body: &'a str,
    pub mentions: &'a [i64],
}

/// 发评论（同一条语句写通知与计数）。楼层已删除、回复的评论无效时返回 `None`；否则返回评论与该楼现在的评论数。
pub async fn create(
    db: impl PgExecutor<'_>,
    c: NewComment<'_>,
) -> Result<Option<(FloorComment, i32)>, sqlx::Error> {
    let row = sqlx::query_file!(
        "queries/comment_create.sql",
        c.post_id,
        c.author_id,
        c.reply_to,
        c.body,
        c.mentions
    )
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| {
        (
            FloorComment {
                id: r.id,
                post_id: r.post_id,
                author_id: r.author_id,
                reply_to_id: r.reply_to_id,
                body: r.body,
                created_at: r.created_at,
            },
            r.comments,
        )
    }))
}

/// 软删除；已删除的不再改。返回该楼现在的评论数，没有改动时为 `None`。
pub async fn soft_delete(
    db: impl PgExecutor<'_>,
    id: i64,
    status: i16,
) -> Result<Option<i32>, sqlx::Error> {
    let row = sqlx::query_file!("queries/comment_soft_delete.sql", id, status)
        .fetch_optional(db)
        .await?;
    Ok(row.map(|r| r.comments))
}

/// 恢复被管理人员删除的评论。返回该楼现在的评论数，没有改动时为 `None`。
pub async fn restore(db: impl PgExecutor<'_>, id: i64) -> Result<Option<i32>, sqlx::Error> {
    let row = sqlx::query_file!("queries/comment_restore.sql", id)
        .fetch_optional(db)
        .await?;
    Ok(row.map(|r| r.comments))
}

/// 一条合并的点赞通知的内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LikeDigest {
    /// 楼主。
    pub to: i64,
    pub topic_id: i64,
    pub post_id: i64,
    pub floor: i32,
    /// 上一条点赞通知之后新增的赞（楼主自己的不算）。
    pub count: i64,
    /// 其中最近的几位，新的在前。
    pub likers: Vec<i64>,
}

/// 这一楼的合并点赞通知进了死信：清掉待发标记，之后的新赞照常排下一条（水位不动，下一条把这些赞一起算上）。
/// 否则标记一直留着，这一楼再也不会有点赞通知。
pub async fn release_like_digest(db: impl PgExecutor<'_>, post_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE post_counters SET like_notify_pending = false WHERE post_id = $1",
        post_id
    )
    .execute(db)
    .await?;
    Ok(())
}

/// 合并通知里列出的点赞人数。
const DIGEST_LIKERS: i32 = 3;

/// worker 投递 `notify.likes` 时调用（在投递事务里）：清掉待发标记（之后的新赞会排下一条），统计水位之后的新赞并推进水位。
/// 楼层已删除、没有新赞（都取消了）时返回 `None`。
pub async fn take_like_digest(
    tx: &mut Transaction<'_, Postgres>,
    post_id: i64,
) -> Result<Option<LikeDigest>, sqlx::Error> {
    // 先锁住计数行：与这一楼并发的点赞在它之后提交，看到待发标记已清掉就会排下一条通知。
    let Some(since) = sqlx::query_scalar!(
        "UPDATE post_counters SET like_notify_pending = false WHERE post_id = $1 RETURNING likes_notified_at",
        post_id
    )
    .fetch_optional(&mut **tx)
    .await?
    else {
        return Ok(None);
    };
    let Some(post) = crate::posts::by_id(&mut **tx, post_id).await? else {
        return Ok(None);
    };
    let fresh = sqlx::query_file!("queries/likes_digest.sql", post_id, since, DIGEST_LIKERS)
        .fetch_one(&mut **tx)
        .await?;
    let Some(newest) = fresh.newest else {
        return Ok(None);
    };
    sqlx::query!(
        "UPDATE post_counters SET likes_notified_at = $2 WHERE post_id = $1",
        post_id,
        newest
    )
    .execute(&mut **tx)
    .await?;
    if post.status != crate::posts::STATUS_NORMAL {
        return Ok(None);
    }
    Ok(Some(LikeDigest {
        to: post.author_id,
        topic_id: post.topic_id,
        post_id,
        floor: post.floor,
        count: fresh.count,
        likers: fresh.likers,
    }))
}
