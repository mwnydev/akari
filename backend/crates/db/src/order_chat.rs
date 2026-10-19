//! 订单内即时通讯（plan 第 7 节）：只追加的哈希链消息、已读位置、建单前谈判记录的关联。
//!
//! 写入（forum-api 的用户消息与 forum-indexer 的系统消息）在事务里先取该订单的事务级咨询锁，
//! 再读链头、算哈希、插入：同一订单的消息严格串行，ID 顺序即链顺序。

pub use forum_core::chat::OrderMessage;
use forum_core::chat::{self, ChainCheck, GENESIS};
use sqlx::{PgExecutor, Postgres, Transaction};

/// 每页条数。
pub const PAGE: i64 = 50;
/// 哈希链校验每批读取条数。
const VERIFY_BATCH: i64 = 1000;

pub const KIND_TEXT: &str = "text";
pub const KIND_SYSTEM: &str = "system";

#[derive(Debug, Clone, Copy)]
pub struct NewOrderMessage<'a> {
    pub order_id: i64,
    /// 系统消息为空。
    pub sender_id: Option<i64>,
    pub kind: &'a str,
    pub body: &'a str,
    pub warn: bool,
}

/// 追加一条消息（必须在事务里，锁随事务结束释放）。
pub async fn append(
    tx: &mut Transaction<'_, Postgres>,
    m: NewOrderMessage<'_>,
) -> Result<OrderMessage, sqlx::Error> {
    sqlx::query!(
        "SELECT pg_advisory_xact_lock(hashtextextended('order_messages', $1))",
        m.order_id
    )
    .execute(&mut **tx)
    .await?;
    let prev = sqlx::query_scalar!(
        "SELECT hash FROM order_messages WHERE order_id = $1 ORDER BY id DESC LIMIT 1",
        m.order_id
    )
    .fetch_optional(&mut **tx)
    .await?
    .unwrap_or_else(|| GENESIS.to_vec());
    let created_at = chat::now_micros();
    let hash = chat::chain_hash(
        &prev,
        &chat::canonical(m.order_id, m.sender_id, m.kind, m.body, None, created_at),
    );
    sqlx::query_as!(
        OrderMessage,
        "INSERT INTO order_messages (order_id, sender_id, kind, body, warn, prev_hash, hash, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         RETURNING id, order_id, sender_id, kind, body, image, warn, prev_hash, hash, created_at",
        m.order_id,
        m.sender_id,
        m.kind,
        m.body,
        m.warn,
        prev,
        hash.as_slice(),
        created_at
    )
    .fetch_one(&mut **tx)
    .await
}

/// 游标之前的一页（新的在前）。
pub async fn before(
    db: impl PgExecutor<'_>,
    order_id: i64,
    before: i64,
    limit: i64,
) -> Result<Vec<OrderMessage>, sqlx::Error> {
    sqlx::query_file_as!(
        OrderMessage,
        "queries/order_messages_before.sql",
        order_id,
        before,
        limit
    )
    .fetch_all(db)
    .await
}

/// 游标之后的一页（按时间正序）。
pub async fn after(
    db: impl PgExecutor<'_>,
    order_id: i64,
    after: i64,
    limit: i64,
) -> Result<Vec<OrderMessage>, sqlx::Error> {
    sqlx::query_file_as!(
        OrderMessage,
        "queries/order_messages_after.sql",
        order_id,
        after,
        limit
    )
    .fetch_all(db)
    .await
}

/// 按 ID 顺序分批读出整条链并重算哈希（plan 第 7 节：裁决前校验）。
pub async fn verify_chain(
    db: impl PgExecutor<'_> + Copy,
    order_id: i64,
) -> Result<ChainCheck, sqlx::Error> {
    let mut check = ChainCheck::default();
    let mut cursor = 0;
    loop {
        let batch = after(db, order_id, cursor, VERIFY_BATCH).await?;
        for m in &batch {
            check.push(m);
        }
        match batch.last() {
            Some(m) if i64::try_from(batch.len()) == Ok(VERIFY_BATCH) => cursor = m.id,
            _ => return Ok(check),
        }
    }
}

pub async fn by_id(db: impl PgExecutor<'_>, id: i64) -> Result<Option<OrderMessage>, sqlx::Error> {
    sqlx::query_as!(
        OrderMessage,
        "SELECT id, order_id, sender_id, kind, body, image, warn, prev_hash, hash, created_at
         FROM order_messages WHERE id = $1",
        id
    )
    .fetch_optional(db)
    .await
}

/// 推进已读位置（不超过最后一条、不后退）。返回推进后的位置；没有变化返回 `None`。
pub async fn mark_read(
    db: impl PgExecutor<'_>,
    order_id: i64,
    user_id: i64,
    last_id: i64,
) -> Result<Option<i64>, sqlx::Error> {
    let row = sqlx::query!(
        r#"WITH head AS (
             SELECT least($3, max(id)) AS id FROM order_messages WHERE order_id = $1
           )
           INSERT INTO order_message_reads AS r (order_id, user_id, last_read_id)
           SELECT $1, $2, head.id FROM head WHERE head.id IS NOT NULL
           ON CONFLICT (order_id, user_id) DO UPDATE SET last_read_id = EXCLUDED.last_read_id, updated_at = now()
             WHERE r.last_read_id < EXCLUDED.last_read_id
           RETURNING last_read_id"#,
        order_id,
        user_id,
        last_id
    )
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| r.last_read_id))
}

/// 各人的已读位置 `(user_id, last_read_id)`。
pub async fn reads(db: impl PgExecutor<'_>, order_id: i64) -> Result<Vec<(i64, i64)>, sqlx::Error> {
    let rows = sqlx::query!(
        "SELECT user_id, last_read_id FROM order_message_reads WHERE order_id = $1 ORDER BY user_id",
        order_id
    )
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.user_id, r.last_read_id))
        .collect())
}

/// 建单前的私信谈判记录：会话与付款上链时的最后一条消息。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Negotiation {
    pub conversation_id: i64,
    pub offer_id: i64,
    pub last_message_id: i64,
}

pub async fn negotiation(
    db: impl PgExecutor<'_>,
    order_id: i64,
) -> Result<Option<Negotiation>, sqlx::Error> {
    sqlx::query_as!(
        Negotiation,
        "SELECT conversation_id, offer_id, last_message_id FROM order_negotiations WHERE order_id = $1",
        order_id
    )
    .fetch_optional(db)
    .await
}

/// 订单由私信报价卡片生成时记下谈判记录（indexer 在付款上链的同一事务里调用）；已有记录或不是私信报价时不变。
pub async fn link_negotiation(db: impl PgExecutor<'_>, order_id: i64) -> Result<bool, sqlx::Error> {
    let done = sqlx::query!(
        "INSERT INTO order_negotiations (order_id, conversation_id, offer_id, last_message_id)
         SELECT o.id, f.conversation_id, f.id, c.last_message_id
         FROM escrow_orders o
         JOIN offers f ON f.offer_hash = o.offer_hash AND f.chain_id = o.chain_id AND f.verifying_contract = o.contract
         JOIN conversations c ON c.id = f.conversation_id
         WHERE o.id = $1 AND c.last_message_id IS NOT NULL
         ON CONFLICT (order_id) DO NOTHING",
        order_id
    )
    .execute(db)
    .await?;
    Ok(done.rows_affected() > 0)
}
