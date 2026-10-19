//! 链上派生表（chains、escrow_events、escrow_orders、indexer_*）。只有 forum-indexer 调用写入函数。
//!
//! 地址、哈希以原始字节传入；金额以十进制文本传入，SQL 里转 numeric。确认级别以文本
//! （`latest` / `safe` / `finalized`）传入，对应枚举 `chain_level`。

use serde_json::Value;
use sqlx::PgExecutor;

use crate::trade::OfferStatus;
use time::OffsetDateTime;

/// 同一条链只允许一个 indexer 写入：在一条专用连接上取会话级咨询锁，连接关闭即释放。
pub async fn try_lock_indexer(db: impl PgExecutor<'_>, chain_id: i64) -> Result<bool, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT pg_try_advisory_lock(hashtextextended('forum-indexer', $1)) AS "locked!""#,
        chain_id
    )
    .fetch_one(db)
    .await?;
    Ok(row.locked)
}

/// 链配置的公开部分。
#[derive(Debug, Clone, Copy)]
pub struct ChainRow<'a> {
    pub chain_id: i64,
    pub name: &'a str,
    pub escrow: &'a [u8],
    pub stream: &'a [u8],
    pub config: &'a [u8],
    pub token: &'a [u8],
    pub token_symbol: &'a str,
    pub token_decimals: i16,
    pub start_block: i64,
    pub safe_depth: i32,
    pub finalized_depth: i32,
}

pub async fn upsert_chain(db: impl PgExecutor<'_>, c: ChainRow<'_>) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO chains (chain_id, name, escrow_address, config_address, token_address, token_symbol,
                             token_decimals, start_block, safe_depth, finalized_depth, stream_address)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
         ON CONFLICT (chain_id) DO UPDATE SET
           name = EXCLUDED.name, escrow_address = EXCLUDED.escrow_address,
           stream_address = EXCLUDED.stream_address,
           config_address = EXCLUDED.config_address, token_address = EXCLUDED.token_address,
           token_symbol = EXCLUDED.token_symbol, token_decimals = EXCLUDED.token_decimals,
           start_block = EXCLUDED.start_block, safe_depth = EXCLUDED.safe_depth,
           finalized_depth = EXCLUDED.finalized_depth, updated_at = now()",
        c.chain_id,
        c.name,
        c.escrow,
        c.config,
        c.token,
        c.token_symbol,
        c.token_decimals,
        c.start_block,
        c.safe_depth,
        c.finalized_depth,
        c.stream
    )
    .execute(db)
    .await?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    pub block_number: i64,
    pub block_hash: Vec<u8>,
    pub safe_block: i64,
    pub finalized_block: i64,
}

pub async fn cursor(db: impl PgExecutor<'_>, chain_id: i64) -> Result<Option<Cursor>, sqlx::Error> {
    sqlx::query_as!(
        Cursor,
        "SELECT block_number, block_hash, safe_block, finalized_block FROM indexer_cursors WHERE chain_id = $1",
        chain_id
    )
    .fetch_optional(db)
    .await
}

/// 移动游标并记一个检查点。
pub async fn set_cursor(
    db: impl PgExecutor<'_>,
    chain_id: i64,
    block_number: i64,
    block_hash: &[u8],
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "WITH cp AS (
           INSERT INTO indexer_checkpoints (chain_id, block_number, block_hash) VALUES ($1, $2, $3)
           ON CONFLICT (chain_id, block_number) DO UPDATE SET block_hash = EXCLUDED.block_hash
         )
         INSERT INTO indexer_cursors (chain_id, block_number, block_hash) VALUES ($1, $2, $3)
         ON CONFLICT (chain_id) DO UPDATE SET
           block_number = EXCLUDED.block_number, block_hash = EXCLUDED.block_hash, updated_at = now()",
        chain_id,
        block_number,
        block_hash
    )
    .execute(db)
    .await?;
    Ok(())
}

/// 记录 safe / finalized 区块号，并清理最终确认之前的检查点（保留不晚于它的最新一个）。
pub async fn set_confirmations(
    db: impl PgExecutor<'_>,
    chain_id: i64,
    safe_block: i64,
    finalized_block: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "WITH pruned AS (
           DELETE FROM indexer_checkpoints
           WHERE chain_id = $1 AND block_number < (
             SELECT max(block_number) FROM indexer_checkpoints WHERE chain_id = $1 AND block_number <= $3)
         )
         UPDATE indexer_cursors SET safe_block = $2, finalized_block = $3, updated_at = now() WHERE chain_id = $1",
        chain_id,
        safe_block,
        finalized_block
    )
    .execute(db)
    .await?;
    Ok(())
}

pub async fn checkpoints(
    db: impl PgExecutor<'_>,
    chain_id: i64,
) -> Result<Vec<(i64, Vec<u8>)>, sqlx::Error> {
    let rows = sqlx::query!(
        "SELECT block_number, block_hash FROM indexer_checkpoints WHERE chain_id = $1",
        chain_id
    )
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.block_number, r.block_hash))
        .collect())
}

/// 受影响的订单（合约地址, 链上订单号）。
pub type OrderRef = (Vec<u8>, i64);

/// 区块回滚：删除 `after` 之后全部未最终确认的事件与检查点，返回受影响的订单。
pub async fn rollback_after(
    db: impl PgExecutor<'_>,
    chain_id: i64,
    after: i64,
) -> Result<Vec<OrderRef>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"WITH del AS (
             DELETE FROM escrow_events
             WHERE chain_id = $1 AND block_number > $2 AND level <> 'finalized'
             RETURNING contract, order_id
           ), cp AS (
             DELETE FROM indexer_checkpoints WHERE chain_id = $1 AND block_number > $2
           )
           SELECT DISTINCT contract AS "contract!", order_id AS "order_id!" FROM del WHERE order_id IS NOT NULL"#,
        chain_id,
        after
    )
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(|r| (r.contract, r.order_id)).collect())
}

/// 库里一条未最终确认的事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEvent {
    pub tx_hash: Vec<u8>,
    pub log_index: i32,
    pub block_hash: Vec<u8>,
}

pub async fn unfinalized_in_range(
    db: impl PgExecutor<'_>,
    chain_id: i64,
    from: i64,
    to: i64,
) -> Result<Vec<StoredEvent>, sqlx::Error> {
    sqlx::query_as!(
        StoredEvent,
        "SELECT tx_hash, log_index, block_hash FROM escrow_events
         WHERE chain_id = $1 AND block_number BETWEEN $2 AND $3 AND level <> 'finalized'",
        chain_id,
        from,
        to
    )
    .fetch_all(db)
    .await
}

/// 按 (tx_hash, log_index) 删除未最终确认的事件，返回受影响的订单。
pub async fn delete_events(
    db: impl PgExecutor<'_>,
    chain_id: i64,
    tx_hashes: &[Vec<u8>],
    log_indexes: &[i32],
) -> Result<Vec<OrderRef>, sqlx::Error> {
    if tx_hashes.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query!(
        r#"WITH del AS (
             DELETE FROM escrow_events e
             USING UNNEST($2::bytea[], $3::int[]) AS k(tx_hash, log_index)
             WHERE e.chain_id = $1 AND e.tx_hash = k.tx_hash AND e.log_index = k.log_index
               AND e.level <> 'finalized'
             RETURNING e.contract, e.order_id
           )
           SELECT DISTINCT contract AS "contract!", order_id AS "order_id!" FROM del WHERE order_id IS NOT NULL"#,
        chain_id,
        tx_hashes,
        log_indexes
    )
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(|r| (r.contract, r.order_id)).collect())
}

#[derive(Debug, Clone, Copy)]
pub struct NewEvent<'a> {
    pub chain_id: i64,
    pub contract: &'a [u8],
    pub block_number: i64,
    pub block_hash: &'a [u8],
    pub block_time: Option<OffsetDateTime>,
    pub tx_hash: &'a [u8],
    pub log_index: i32,
    pub name: &'a str,
    pub order_id: Option<i64>,
    pub topics: &'a [Vec<u8>],
    pub data: &'a [u8],
    pub payload: &'a Value,
}

/// 写入一条事件（级别 latest），返回是否有改动。
///
/// 同一日志（同一交易同一序号）已存在且区块哈希不同：`replace` 为真（校对补拉的主链数据）时改成新区块，
/// 否则（WebSocket 推送，不可靠）保留原行。已最终确认的行永远不改。
pub async fn insert_event(
    db: impl PgExecutor<'_>,
    e: NewEvent<'_>,
    replace: bool,
) -> Result<bool, sqlx::Error> {
    let r = sqlx::query!(
        "INSERT INTO escrow_events (chain_id, contract, block_number, block_hash, block_time, tx_hash, log_index,
                                    name, order_id, topics, data, payload)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
         ON CONFLICT (chain_id, tx_hash, log_index) DO UPDATE SET
           contract = EXCLUDED.contract, block_number = EXCLUDED.block_number, block_hash = EXCLUDED.block_hash,
           block_time = EXCLUDED.block_time, name = EXCLUDED.name, order_id = EXCLUDED.order_id,
           level = 'latest', topics = EXCLUDED.topics, data = EXCLUDED.data, payload = EXCLUDED.payload
         WHERE $13 AND escrow_events.block_hash <> EXCLUDED.block_hash AND escrow_events.level <> 'finalized'",
        e.chain_id,
        e.contract,
        e.block_number,
        e.block_hash,
        e.block_time,
        e.tx_hash,
        e.log_index,
        e.name,
        e.order_id,
        e.topics,
        e.data,
        e.payload,
        replace
    )
    .execute(db)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// 折叠一个订单所需的事件字段。
#[derive(Debug, Clone)]
pub struct OrderEventRow {
    pub block_number: i64,
    pub log_index: i32,
    pub block_time: Option<OffsetDateTime>,
    pub level: String,
    pub topics: Vec<Vec<u8>>,
    pub data: Vec<u8>,
    pub snapshot: Option<Value>,
}

pub async fn order_events(
    db: impl PgExecutor<'_>,
    chain_id: i64,
    contract: &[u8],
    order_id: i64,
) -> Result<Vec<OrderEventRow>, sqlx::Error> {
    sqlx::query_as!(
        OrderEventRow,
        r#"SELECT block_number, log_index, block_time, level::text AS "level!", topics, data,
                  payload->'snapshot' AS snapshot
           FROM escrow_events
           WHERE chain_id = $1 AND contract = $2 AND order_id = $3
           ORDER BY block_number, log_index"#,
        chain_id,
        contract,
        order_id
    )
    .fetch_all(db)
    .await
}

/// 折叠后的一行订单。金额为十进制文本。
#[derive(Debug, Clone, Default)]
pub struct OrderUpsert<'a> {
    pub chain_id: i64,
    pub contract: &'a [u8],
    pub chain_order_id: i64,
    /// 0 = 一次性担保、1 = 分期。
    pub mode: i16,
    pub buyer: Vec<u8>,
    pub seller: Vec<u8>,
    pub token: Vec<u8>,
    pub amount: String,
    pub buyer_fee: Option<String>,
    pub seller_fee: Option<String>,
    pub buyer_gas: Option<String>,
    pub seller_gas: Option<String>,
    pub dispute_fee: Option<String>,
    pub bond: Option<String>,
    pub instant: bool,
    pub offer_hash: Vec<u8>,
    pub listing_hash: Vec<u8>,
    pub auction_id: Option<String>,
    pub state: i16,
    pub ship_deadline: Option<OffsetDateTime>,
    pub confirm_deadline: Option<OffsetDateTime>,
    pub proposer: Option<Vec<u8>>,
    pub proposal_to_buyer: Option<String>,
    pub proposal_expiry: Option<OffsetDateTime>,
    pub disputed_by: Option<Vec<u8>>,
    pub dispute_from_bond: Option<bool>,
    pub disputed_at: Option<OffsetDateTime>,
    pub ruling_to_buyer_bps: Option<i32>,
    pub ruling_bond_to_buyer_bps: Option<i32>,
    pub settled_to_buyer: Option<String>,
    pub settled_to_seller: Option<String>,
    pub settled_to_fee: Option<String>,
    pub interval_secs: Option<i32>,
    pub periods: Option<i32>,
    pub started_at: Option<OffsetDateTime>,
    /// 分期已领取（合约 `Order.claimed`）；一次性担保为 `None`（存 0）。
    pub released_amount: Option<String>,
    pub snapshot: Option<Value>,
    pub level: &'a str,
    pub created_block: i64,
    pub updated_block: i64,
}

/// 订单行变化后的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderChanged {
    pub id: i64,
    pub buyer_id: Option<i64>,
    pub seller_id: Option<i64>,
    pub offer_hash: Vec<u8>,
}

/// 写入订单；内容没变时返回 `None`。
pub async fn upsert_order(
    db: impl PgExecutor<'_>,
    o: &OrderUpsert<'_>,
) -> Result<Option<OrderChanged>, sqlx::Error> {
    sqlx::query_file_as!(
        OrderChanged,
        "queries/escrow_order_upsert.sql",
        o.chain_id,
        o.contract,
        o.chain_order_id,
        o.mode,
        o.buyer,
        o.seller,
        o.token,
        o.amount,
        o.buyer_fee,
        o.seller_fee,
        o.buyer_gas,
        o.seller_gas,
        o.dispute_fee,
        o.bond,
        o.instant,
        o.offer_hash,
        o.listing_hash,
        o.auction_id,
        o.state,
        o.ship_deadline,
        o.confirm_deadline,
        o.proposer,
        o.proposal_to_buyer,
        o.proposal_expiry,
        o.disputed_by,
        o.dispute_from_bond,
        o.disputed_at,
        o.ruling_to_buyer_bps,
        o.ruling_bond_to_buyer_bps,
        o.settled_to_buyer,
        o.settled_to_seller,
        o.settled_to_fee,
        o.interval_secs,
        o.periods,
        o.started_at,
        o.released_amount,
        o.snapshot,
        o.level,
        o.created_block,
        o.updated_block
    )
    .fetch_optional(db)
    .await
}

/// 下单事件已被回滚：保留行（id 不变），状态置 0。已经是 0 或不存在时返回 `None`。
pub async fn clear_order(
    db: impl PgExecutor<'_>,
    chain_id: i64,
    contract: &[u8],
    chain_order_id: i64,
) -> Result<Option<OrderChanged>, sqlx::Error> {
    sqlx::query_as!(
        OrderChanged,
        "UPDATE escrow_orders SET state = 0, level = 'latest', updated_at = now()
         WHERE chain_id = $1 AND contract = $2 AND chain_order_id = $3 AND state <> 0
         RETURNING id, buyer_id, seller_id, offer_hash",
        chain_id,
        contract,
        chain_order_id
    )
    .fetch_optional(db)
    .await
}

/// 状态随链上成交改变的报价卡片。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OfferFill {
    pub id: i64,
    pub conversation_id: Option<i64>,
    pub seller_id: i64,
    pub buyer_id: Option<i64>,
    pub status: OfferStatus,
}

/// 订单行变化后同步生成它的报价卡片（见 `queries/offer_fills_sync.sql`）。返回状态改变的卡片。
pub async fn sync_offer_fills(
    db: impl PgExecutor<'_>,
    chain_id: i64,
    contract: &[u8],
    offer_hash: &[u8],
) -> Result<Vec<OfferFill>, sqlx::Error> {
    sqlx::query_file_as!(
        OfferFill,
        "queries/offer_fills_sync.sql",
        chain_id,
        contract,
        offer_hash
    )
    .fetch_all(db)
    .await
}

/// 把 `upto` 及之前、级别低于 `level` 的事件升级为 `level`，返回受影响的订单。
pub async fn promote(
    db: impl PgExecutor<'_>,
    chain_id: i64,
    upto: i64,
    level: &str,
) -> Result<Vec<OrderRef>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"WITH up AS (
             UPDATE escrow_events SET level = $3::text::chain_level
             WHERE chain_id = $1 AND block_number <= $2 AND level < $3::text::chain_level
             RETURNING contract, order_id
           )
           SELECT DISTINCT contract AS "contract!", order_id AS "order_id!" FROM up WHERE order_id IS NOT NULL"#,
        chain_id,
        upto,
        level
    )
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(|r| (r.contract, r.order_id)).collect())
}

/// 订单聊天系统消息要比较的字段（indexer 在改写订单行之前读取旧值）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatBrief {
    pub state: i16,
    pub buyer: Vec<u8>,
    pub instant: bool,
    pub confirm_deadline: Option<OffsetDateTime>,
    pub started_at: Option<OffsetDateTime>,
    pub proposer: Option<Vec<u8>>,
    pub proposal_to_buyer: Option<String>,
    pub disputed_by: Option<Vec<u8>>,
}

pub async fn chat_brief(
    db: impl PgExecutor<'_>,
    chain_id: i64,
    contract: &[u8],
    chain_order_id: i64,
) -> Result<Option<ChatBrief>, sqlx::Error> {
    sqlx::query_as!(
        ChatBrief,
        r#"SELECT state, buyer, instant, confirm_deadline, started_at, proposer,
                  proposal_to_buyer::text AS "proposal_to_buyer?", disputed_by
           FROM escrow_orders WHERE chain_id = $1 AND contract = $2 AND chain_order_id = $3"#,
        chain_id,
        contract,
        chain_order_id
    )
    .fetch_optional(db)
    .await
}
