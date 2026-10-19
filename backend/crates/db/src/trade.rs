//! 交易帖与卖家报价卡片（plan 第 6、7 节）。
//!
//! 金额列是 `NUMERIC`：进出都用十进制字符串（`$n::text::numeric` / `col::text`），不引入十进制库。

use serde_json::Value;
use sqlx::PgExecutor;
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "offer_status", rename_all = "lowercase")]
pub enum OfferStatus {
    /// 待对方确认
    Pending,
    /// 已修改（被新卡片替换）
    Modified,
    Rejected,
    Cancelled,
    Expired,
    /// 买家已付款建单（indexer 写入）
    Paid,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Listing {
    pub id: i64,
    pub topic_id: i64,
    pub seller_id: i64,
    pub chain_id: i64,
    pub price: String,
    pub terms: Value,
    pub content_hash: Vec<u8>,
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Clone, Copy)]
pub struct NewListing<'a> {
    pub topic_id: i64,
    pub seller_id: i64,
    pub chain_id: i64,
    pub price: &'a str,
    pub terms: &'a Value,
    pub content_hash: &'a [u8],
}

pub async fn insert_listing(
    db: impl PgExecutor<'_>,
    l: NewListing<'_>,
) -> Result<i64, sqlx::Error> {
    let row = sqlx::query!(
        r#"INSERT INTO trade_listings (topic_id, seller_id, chain_id, price, terms, content_hash)
           VALUES ($1, $2, $3, $4::text::numeric, $5, $6) RETURNING id"#,
        l.topic_id,
        l.seller_id,
        l.chain_id,
        l.price,
        l.terms,
        l.content_hash
    )
    .fetch_one(db)
    .await?;
    Ok(row.id)
}

pub async fn listing(db: impl PgExecutor<'_>, id: i64) -> Result<Option<Listing>, sqlx::Error> {
    sqlx::query_as!(
        Listing,
        r#"SELECT id, topic_id, seller_id, chain_id, price::text AS "price!", terms, content_hash, created_at
           FROM trade_listings WHERE id = $1"#,
        id
    )
    .fetch_optional(db)
    .await
}

/// 报价卡片的一行。字段与 `Escrow.Offer` 同名，金额为十进制字符串。
#[derive(Debug, Clone, PartialEq)]
pub struct OfferRow {
    pub id: i64,
    pub listing_id: Option<i64>,
    pub conversation_id: Option<i64>,
    pub seller_id: i64,
    pub buyer_id: Option<i64>,
    pub replaces_id: Option<i64>,
    pub chain_id: i64,
    pub verifying_contract: Vec<u8>,
    pub seller: Vec<u8>,
    pub buyer: Vec<u8>,
    pub token: Vec<u8>,
    pub amount: String,
    pub fee_payer: i16,
    pub max_fee_bps: i32,
    pub max_base_gas: String,
    pub max_min_fee: String,
    pub bond: String,
    pub late_slash_bps: i32,
    pub instant: bool,
    pub ship_window: i64,
    pub confirm_window: i64,
    pub listing_hash: Vec<u8>,
    pub expiry: i64,
    pub max_fills: i64,
    pub nonce: String,
    pub signature: Vec<u8>,
    pub offer_hash: Vec<u8>,
    pub status: OfferStatus,
    pub reject_reason: Option<String>,
    pub expires_at: OffsetDateTime,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// 新报价。`expires_at` 由 `expiry` 推出，不单独传。
#[derive(Debug, Clone, Copy)]
pub struct NewOffer<'a> {
    pub listing_id: Option<i64>,
    pub conversation_id: Option<i64>,
    pub seller_id: i64,
    pub buyer_id: Option<i64>,
    pub replaces_id: Option<i64>,
    pub chain_id: i64,
    pub verifying_contract: &'a [u8],
    pub seller: &'a [u8],
    pub buyer: &'a [u8],
    pub token: &'a [u8],
    pub amount: &'a str,
    pub fee_payer: i16,
    pub max_fee_bps: i32,
    pub max_base_gas: &'a str,
    pub max_min_fee: &'a str,
    pub bond: &'a str,
    pub late_slash_bps: i32,
    pub instant: bool,
    pub ship_window: i64,
    pub confirm_window: i64,
    pub listing_hash: &'a [u8],
    pub expiry: i64,
    pub max_fills: i64,
    pub nonce: &'a str,
    pub signature: &'a [u8],
    pub offer_hash: &'a [u8],
}

/// 插入报价；同一报价哈希已存在返回 `None`。
pub async fn insert_offer(
    db: impl PgExecutor<'_>,
    o: NewOffer<'_>,
) -> Result<Option<i64>, sqlx::Error> {
    let row = sqlx::query!(
        r#"INSERT INTO offers (
             listing_id, conversation_id, seller_id, buyer_id, replaces_id, chain_id, verifying_contract,
             seller, buyer, token, amount, fee_payer, max_fee_bps, max_base_gas, max_min_fee, bond,
             late_slash_bps, instant, ship_window, confirm_window, listing_hash, expiry, max_fills, nonce,
             signature, offer_hash, expires_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11::text::numeric, $12, $13, $14::text::numeric,
             $15::text::numeric, $16::text::numeric, $17, $18, $19, $20, $21, $22, $23, $24::text::numeric,
             $25, $26, to_timestamp($22::bigint))
           ON CONFLICT (offer_hash) DO NOTHING
           RETURNING id"#,
        o.listing_id,
        o.conversation_id,
        o.seller_id,
        o.buyer_id,
        o.replaces_id,
        o.chain_id,
        o.verifying_contract,
        o.seller,
        o.buyer,
        o.token,
        o.amount,
        o.fee_payer,
        o.max_fee_bps,
        o.max_base_gas,
        o.max_min_fee,
        o.bond,
        o.late_slash_bps,
        o.instant,
        o.ship_window,
        o.confirm_window,
        o.listing_hash,
        o.expiry,
        o.max_fills,
        o.nonce,
        o.signature,
        o.offer_hash
    )
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| r.id))
}

/// 列表每次最多返回的卡片数。
pub const OFFERS_PAGE: i64 = 50;

pub async fn offer(db: impl PgExecutor<'_>, id: i64) -> Result<Option<OfferRow>, sqlx::Error> {
    sqlx::query_file_as!(OfferRow, "queries/offers_by_id.sql", id)
        .fetch_optional(db)
        .await
}

/// 交易帖下的报价，新的在前。公开报价人人可见；指定买家的报价只有买卖双方可见。
pub async fn listing_offers(
    db: impl PgExecutor<'_>,
    listing_id: i64,
    viewer: Option<i64>,
) -> Result<Vec<OfferRow>, sqlx::Error> {
    sqlx::query_file_as!(
        OfferRow,
        "queries/offers_listing.sql",
        listing_id,
        viewer,
        OFFERS_PAGE
    )
    .fetch_all(db)
    .await
}

/// 私信会话里的报价卡片，只有买卖双方可见。
pub async fn conversation_offers(
    db: impl PgExecutor<'_>,
    conversation_id: i64,
    viewer: i64,
) -> Result<Vec<OfferRow>, sqlx::Error> {
    sqlx::query_file_as!(
        OfferRow,
        "queries/offers_conversation.sql",
        conversation_id,
        viewer,
        OFFERS_PAGE
    )
    .fetch_all(db)
    .await
}

/// 把仍待确认且未过期的卡片改成 `to`（只有待确认卡片能变，保证并发下只有一次状态变化成功）。
/// 返回是否改动成功。
pub async fn close_offer(
    db: impl PgExecutor<'_>,
    id: i64,
    to: OfferStatus,
    reason: Option<&str>,
) -> Result<bool, sqlx::Error> {
    let row = sqlx::query!(
        r#"UPDATE offers SET status = $2, reject_reason = $3, updated_at = now()
           WHERE id = $1 AND status = 'pending' AND expires_at > now()
           RETURNING id"#,
        id,
        to as OfferStatus,
        reason
    )
    .fetch_optional(db)
    .await?;
    Ok(row.is_some())
}

/// worker 定时任务：到期的待确认卡片标为已过期，返回本批条数。
pub async fn expire_due(db: impl PgExecutor<'_>, limit: i64) -> Result<u64, sqlx::Error> {
    let done = sqlx::query_file!("queries/offers_expire.sql", limit)
        .execute(db)
        .await?;
    Ok(done.rows_affected())
}
