//! 订单（escrow_orders，由 forum-indexer 写入）的读取：我的订单、订单详情（plan 第 7、8 节）。
//!
//! 列表先按 `(buyer_id, id DESC)` / `(seller_id, id DESC)` 索引取一页 ID，再按主键批量取整行。

use sqlx::PgExecutor;
use time::OffsetDateTime;

/// 列表每页条数。
pub const ORDERS_PAGE: i64 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Buyer,
    Seller,
}

/// 订单的一行。金额为十进制文本；地址、哈希为原始字节。
#[derive(Debug, Clone, PartialEq)]
pub struct OrderRow {
    pub id: i64,
    pub chain_id: i64,
    pub contract: Vec<u8>,
    pub chain_order_id: i64,
    pub mode: i16,
    pub buyer: Vec<u8>,
    pub seller: Vec<u8>,
    pub buyer_id: Option<i64>,
    pub seller_id: Option<i64>,
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
    pub released_amount: String,
    pub level: String,
    pub created_block: i64,
    pub updated_block: i64,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    /// 生成该订单的站内报价卡片（拍卖成交或站外报价没有）。
    pub offer_id: Option<i64>,
    pub listing_id: Option<i64>,
    pub listing_topic_id: Option<i64>,
}

/// 我作为买家或卖家的订单，`before` 之前（不含）的一页 ID，新的在前。
pub async fn page_ids(
    db: impl PgExecutor<'_>,
    role: Role,
    user_id: i64,
    before: i64,
    limit: i64,
) -> Result<Vec<i64>, sqlx::Error> {
    match role {
        Role::Buyer => {
            sqlx::query_file_scalar!("queries/escrow_orders_buyer.sql", user_id, before, limit)
                .fetch_all(db)
                .await
        }
        Role::Seller => {
            sqlx::query_file_scalar!("queries/escrow_orders_seller.sql", user_id, before, limit)
                .fetch_all(db)
                .await
        }
    }
}

/// 按 ID 批量取订单，新的在前；不存在的 ID 跳过。
pub async fn by_ids(db: impl PgExecutor<'_>, ids: &[i64]) -> Result<Vec<OrderRow>, sqlx::Error> {
    sqlx::query_file_as!(OrderRow, "queries/escrow_orders_by_ids.sql", ids)
        .fetch_all(db)
        .await
}

/// 报价卡片生成的、`user_id` 参与的最新订单。
pub async fn id_by_offer(
    db: impl PgExecutor<'_>,
    offer_hash: &[u8],
    chain_id: i64,
    contract: &[u8],
    user_id: i64,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_file_scalar!(
        "queries/escrow_order_by_offer.sql",
        offer_hash,
        chain_id,
        contract,
        user_id
    )
    .fetch_optional(db)
    .await
}
