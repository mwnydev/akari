//! 私信（plan 第 4、7 节）：一对一会话、消息、已读位置、屏蔽。
//!
//! 会话按 `(user_low, user_high)` 唯一；写消息的事务先锁会话行，同一会话的消息串行写入，
//! `last_message_id` 与各自的已读位置随之更新（发送者自己的消息直接算已读）。

pub use forum_core::chat::Message;
use sqlx::{PgExecutor, Postgres, Transaction};
use time::OffsetDateTime;

/// 消息每页条数。
pub const MESSAGES_PAGE: i64 = 50;
/// 会话列表每页条数。
pub const CONVERSATIONS_PAGE: i64 = 20;
/// 未读数最多数到这里（界面显示 99+）。
pub const UNREAD_CAP: i64 = 100;

pub const KIND_TEXT: &str = "text";
pub const KIND_OFFER: &str = "offer";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Conversation {
    pub id: i64,
    pub user_low: i64,
    pub user_high: i64,
    pub low_read_id: i64,
    pub high_read_id: i64,
    pub last_message_id: Option<i64>,
    pub created_at: OffsetDateTime,
}

impl Conversation {
    pub fn is_member(&self, user_id: i64) -> bool {
        self.user_low == user_id || self.user_high == user_id
    }

    /// 对方；`user_id` 必须是成员。
    pub fn peer(&self, user_id: i64) -> i64 {
        if self.user_low == user_id {
            self.user_high
        } else {
            self.user_low
        }
    }

    pub fn read_id(&self, user_id: i64) -> i64 {
        if self.user_low == user_id {
            self.low_read_id
        } else {
            self.high_read_id
        }
    }

    pub fn members(&self) -> [i64; 2] {
        [self.user_low, self.user_high]
    }
}

pub async fn conversation(
    db: impl PgExecutor<'_>,
    id: i64,
) -> Result<Option<Conversation>, sqlx::Error> {
    sqlx::query_as!(
        Conversation,
        "SELECT id, user_low, user_high, low_read_id, high_read_id, last_message_id, created_at
         FROM conversations WHERE id = $1",
        id
    )
    .fetch_optional(db)
    .await
}

/// 在事务里锁住会话行：同一会话的消息按提交顺序写入，`last_message_id` 不会倒退。
pub async fn lock_conversation(
    tx: &mut Transaction<'_, Postgres>,
    id: i64,
) -> Result<Option<Conversation>, sqlx::Error> {
    sqlx::query_as!(
        Conversation,
        "SELECT id, user_low, user_high, low_read_id, high_read_id, last_message_id, created_at
         FROM conversations WHERE id = $1 FOR UPDATE",
        id
    )
    .fetch_optional(&mut **tx)
    .await
}

/// 两人之间的会话。
pub async fn between(
    db: impl PgExecutor<'_>,
    a: i64,
    b: i64,
) -> Result<Option<Conversation>, sqlx::Error> {
    sqlx::query_as!(
        Conversation,
        "SELECT id, user_low, user_high, low_read_id, high_read_id, last_message_id, created_at
         FROM conversations WHERE user_low = $1 AND user_high = $2",
        a.min(b),
        a.max(b)
    )
    .fetch_optional(db)
    .await
}

/// 两人之间的会话，不存在则创建。返回 `(会话, 是否新建)`。`a != b` 由调用方保证。
pub async fn get_or_create(
    db: &sqlx::PgPool,
    a: i64,
    b: i64,
) -> Result<(Conversation, bool), sqlx::Error> {
    if let Some(c) = between(db, a, b).await? {
        return Ok((c, false));
    }
    let created = sqlx::query_as!(
        Conversation,
        "INSERT INTO conversations (user_low, user_high) VALUES ($1, $2)
         ON CONFLICT (user_low, user_high) DO NOTHING
         RETURNING id, user_low, user_high, low_read_id, high_read_id, last_message_id, created_at",
        a.min(b),
        a.max(b)
    )
    .fetch_optional(db)
    .await?;
    match created {
        Some(c) => Ok((c, true)),
        // 并发创建：对方刚插入，再读一次。
        None => between(db, a, b)
            .await?
            .map(|c| (c, false))
            .ok_or(sqlx::Error::RowNotFound),
    }
}

/// 会话列表里的一项。
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub id: i64,
    pub peer_id: i64,
    pub my_read_id: i64,
    pub peer_read_id: i64,
    pub last_message: Message,
    /// 对方发来、我还没读的消息数（最多 [`UNREAD_CAP`]）。
    pub unread: i64,
    /// 我屏蔽了对方。
    pub blocked: bool,
}

/// 会话列表，`before`（会话的 `last_message_id`）之前的一页，最近的在前。
pub async fn list(
    db: impl PgExecutor<'_>,
    user_id: i64,
    before: i64,
    limit: i64,
) -> Result<Vec<Summary>, sqlx::Error> {
    let rows = sqlx::query_file!("queries/conversations_list.sql", user_id, before, limit)
        .fetch_all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| Summary {
            id: r.id,
            peer_id: r.peer_id,
            my_read_id: r.my_read_id,
            peer_read_id: r.peer_read_id,
            last_message: Message {
                id: r.message_id,
                conversation_id: r.id,
                sender_id: r.sender_id,
                kind: r.kind,
                body: r.body,
                image: r.image,
                offer_id: r.offer_id,
                warn: r.warn,
                created_at: r.created_at,
            },
            unread: r.unread,
            blocked: r.blocked,
        })
        .collect())
}

/// 游标之前的一页（新的在前）。
pub async fn before(
    db: impl PgExecutor<'_>,
    conversation_id: i64,
    before: i64,
    limit: i64,
) -> Result<Vec<Message>, sqlx::Error> {
    sqlx::query_file_as!(
        Message,
        "queries/messages_before.sql",
        conversation_id,
        before,
        limit
    )
    .fetch_all(db)
    .await
}

/// 游标之后的一页（按时间正序）。
pub async fn after(
    db: impl PgExecutor<'_>,
    conversation_id: i64,
    after: i64,
    limit: i64,
) -> Result<Vec<Message>, sqlx::Error> {
    sqlx::query_file_as!(
        Message,
        "queries/messages_after.sql",
        conversation_id,
        after,
        limit
    )
    .fetch_all(db)
    .await
}

pub async fn by_id(db: impl PgExecutor<'_>, id: i64) -> Result<Option<Message>, sqlx::Error> {
    sqlx::query_as!(
        Message,
        "SELECT id, conversation_id, sender_id, kind, body, image, offer_id, warn, created_at
         FROM messages WHERE id = $1",
        id
    )
    .fetch_optional(db)
    .await
}

#[derive(Debug, Clone, Copy)]
pub struct NewMessage<'a> {
    pub conversation_id: i64,
    pub sender_id: Option<i64>,
    pub kind: &'a str,
    pub body: &'a str,
    pub offer_id: Option<i64>,
    pub warn: bool,
}

/// 写入一条消息并推进会话的 `last_message_id`；发送者的已读位置同时推进到这条。
pub async fn insert(db: impl PgExecutor<'_>, m: NewMessage<'_>) -> Result<Message, sqlx::Error> {
    sqlx::query_as!(
        Message,
        r#"WITH m AS (
             INSERT INTO messages (conversation_id, sender_id, kind, body, offer_id, warn)
             VALUES ($1, $2, $3, $4, $5, $6)
             RETURNING id, conversation_id, sender_id, kind, body, image, offer_id, warn, created_at
           ), c AS (
             UPDATE conversations SET
               last_message_id = greatest(coalesce(last_message_id, 0), m.id),
               low_read_id = CASE WHEN user_low = m.sender_id THEN m.id ELSE low_read_id END,
               high_read_id = CASE WHEN user_high = m.sender_id THEN m.id ELSE high_read_id END
             FROM m WHERE conversations.id = m.conversation_id
           )
           SELECT id AS "id!", conversation_id AS "conversation_id!", sender_id, kind AS "kind!", body AS "body!",
                  image, offer_id, warn AS "warn!", created_at AS "created_at!"
           FROM m"#,
        m.conversation_id,
        m.sender_id,
        m.kind,
        m.body,
        m.offer_id,
        m.warn
    )
    .fetch_one(db)
    .await
}

/// 把 `user_id` 的已读位置推进到 `last_id`（不超过会话最后一条、不后退）。返回推进后的位置；没有变化返回 `None`。
pub async fn mark_read(
    db: impl PgExecutor<'_>,
    conversation_id: i64,
    user_id: i64,
    last_id: i64,
) -> Result<Option<i64>, sqlx::Error> {
    let row = sqlx::query!(
        r#"UPDATE conversations SET
             low_read_id = CASE WHEN user_low = $2 THEN least($3, last_message_id) ELSE low_read_id END,
             high_read_id = CASE WHEN user_high = $2 THEN least($3, last_message_id) ELSE high_read_id END
           WHERE id = $1 AND (user_low = $2 OR user_high = $2) AND last_message_id IS NOT NULL
             AND least($3, last_message_id) > CASE WHEN user_low = $2 THEN low_read_id ELSE high_read_id END
           RETURNING CASE WHEN user_low = $2 THEN low_read_id ELSE high_read_id END AS "read_id!""#,
        conversation_id,
        user_id,
        last_id
    )
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| r.read_id))
}

/// `blocker` 屏蔽 `blocked`（重复屏蔽不报错）。
pub async fn block(db: impl PgExecutor<'_>, blocker: i64, blocked: i64) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO user_blocks (blocker_id, blocked_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        blocker,
        blocked
    )
    .execute(db)
    .await?;
    Ok(())
}

pub async fn unblock(
    db: impl PgExecutor<'_>,
    blocker: i64,
    blocked: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "DELETE FROM user_blocks WHERE blocker_id = $1 AND blocked_id = $2",
        blocker,
        blocked
    )
    .execute(db)
    .await?;
    Ok(())
}

/// 两人之间任一方屏蔽了另一方。
pub async fn blocked_between(db: impl PgExecutor<'_>, a: i64, b: i64) -> Result<bool, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT EXISTS (
             SELECT 1 FROM user_blocks
             WHERE (blocker_id = $1 AND blocked_id = $2) OR (blocker_id = $2 AND blocked_id = $1)
           ) AS "blocked!""#,
        a,
        b
    )
    .fetch_one(db)
    .await?;
    Ok(row.blocked)
}

/// 我是否屏蔽了对方。
pub async fn has_blocked(
    db: impl PgExecutor<'_>,
    blocker: i64,
    blocked: i64,
) -> Result<bool, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT EXISTS (SELECT 1 FROM user_blocks WHERE blocker_id = $1 AND blocked_id = $2) AS "blocked!""#,
        blocker,
        blocked
    )
    .fetch_one(db)
    .await?;
    Ok(row.blocked)
}

/// 对方发来、`user_id` 在 `read_id` 之后还没读的消息数（最多 [`UNREAD_CAP`]）。
pub async fn unread(
    db: impl PgExecutor<'_>,
    conversation_id: i64,
    user_id: i64,
    read_id: i64,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!" FROM (
             SELECT 1 FROM messages
             WHERE conversation_id = $1 AND id > $3 AND sender_id IS DISTINCT FROM $2
             LIMIT $4
           ) s"#,
        conversation_id,
        user_id,
        read_id,
        UNREAD_CAP
    )
    .fetch_one(db)
    .await
}
