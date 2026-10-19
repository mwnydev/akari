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
    /// 私信报价生成的订单：谈判会话与付款上链时的最后一条消息（仲裁证据）。
    pub negotiation_conversation_id: Option<i64>,
    pub negotiation_last_message_id: Option<i64>,
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

/// `Escrow.State` 里进行中的两种状态。
pub const STATE_FUNDED: i16 = 1;
pub const STATE_DISPUTED: i16 = 2;

/// 订单的参与方（聊天与推送的权限判断）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parties {
    pub id: i64,
    pub buyer_id: Option<i64>,
    pub seller_id: Option<i64>,
    pub state: i16,
}

impl Parties {
    pub fn is_party(&self, user_id: i64) -> bool {
        self.buyer_id == Some(user_id) || self.seller_id == Some(user_id)
    }

    /// 本站用户的买卖双方。
    pub fn users(&self) -> Vec<i64> {
        self.buyer_id.into_iter().chain(self.seller_id).collect()
    }

    /// 进行中（已付款或争议中）。
    pub fn open(&self) -> bool {
        matches!(self.state, STATE_FUNDED | STATE_DISPUTED)
    }
}

pub async fn parties(db: impl PgExecutor<'_>, id: i64) -> Result<Option<Parties>, sqlx::Error> {
    sqlx::query_as!(
        Parties,
        "SELECT id, buyer_id, seller_id, state FROM escrow_orders WHERE id = $1",
        id
    )
    .fetch_optional(db)
    .await
}

/// `ids` 里与 `viewer` 有进行中订单的人（隐身用户对交易对手仍显示真实在线状态）。
pub async fn open_counterparties(
    db: impl PgExecutor<'_>,
    viewer: i64,
    ids: &[i64],
) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT seller_id AS "id!" FROM escrow_orders
           WHERE buyer_id = $1 AND seller_id = ANY($2::bigint[]) AND state IN (1, 2)
           UNION
           SELECT buyer_id FROM escrow_orders
           WHERE seller_id = $1 AND buyer_id = ANY($2::bigint[]) AND state IN (1, 2)"#,
        viewer,
        ids
    )
    .fetch_all(db)
    .await
}
