//! 订单（plan 第 7、8 节「我的订单」）：数据全部来自 forum-indexer 写入的 escrow_orders，这里只读。
//!
//! 只有买卖双方和该订单仲裁案件的仲裁员、站长能看订单，其他人一律 404（不暴露订单是否存在）。金额为 USDT 最小单位的十进制字符串，
//! 时间为 RFC 3339；`level` 是订单全部事件中最低的确认级别（latest / safe / finalized）。

use axum::Json;
use axum::extract::{Path, Query, State};
use forum_chain::order::{Market, OrderState};
use forum_core::ErrorCode;
use forum_db::orders::{self as db, OrderRow, Role};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};
use tower_sessions::Session;
use utoipa::{IntoParams, ToSchema};

use super::me::current_user_id;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::state::AppState;
use crate::views::hex0x;

#[derive(Debug, Clone, Copy, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum OrderRole {
    Buyer,
    Seller,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct OrdersQuery {
    /// 我作为买家还是卖家的订单。
    #[param(inline)]
    pub role: OrderRole,
    /// 上一页的 `next_cursor`。
    pub cursor: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OrderPage {
    pub orders: Vec<OrderView>,
    /// 还有下一页时为下一页的游标。
    pub next_cursor: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Party {
    pub address: String,
    /// 钱包属于本站用户时。
    pub user_id: Option<i64>,
}

/// 下单时固定的费用（下单区块 `getOrder` 快照；快照缺失时为空）。
#[derive(Debug, Serialize, ToSchema)]
pub struct OrderFees {
    pub buyer_fee: Option<String>,
    pub seller_fee: Option<String>,
    pub buyer_gas: Option<String>,
    pub seller_gas: Option<String>,
    pub dispute_fee: Option<String>,
    pub bond: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Deadlines {
    /// 一次性：发货期限（即时发货为空）。
    #[schema(value_type = Option<String>, format = DateTime)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub ship: Option<OffsetDateTime>,
    /// 一次性：买家确认期限（发货后才有）。
    #[schema(value_type = Option<String>, format = DateTime)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub confirm: Option<OffsetDateTime>,
    /// 分期：卖家开始服务的期限。
    #[schema(value_type = Option<String>, format = DateTime)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub start: Option<OffsetDateTime>,
}

/// 分期订单的进度。
#[derive(Debug, Serialize, ToSchema)]
pub struct StreamView {
    pub interval_secs: i32,
    pub periods: i32,
    #[schema(value_type = Option<String>, format = DateTime)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub started_at: Option<OffsetDateTime>,
    /// 全部期数到期的时间（开始后才有）。
    #[schema(value_type = Option<String>, format = DateTime)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub ends_at: Option<OffsetDateTime>,
    /// 已领取的可解锁净本金（含领取 gas 与从已解锁部分扣的争议费）。
    pub claimed: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProposalView {
    pub proposer: String,
    pub to_buyer: String,
    #[schema(value_type = Option<String>, format = DateTime)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub expiry: Option<OffsetDateTime>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DisputeView {
    pub by: String,
    /// 争议费从卖家保证金扣。
    pub from_bond: bool,
    #[schema(value_type = Option<String>, format = DateTime)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub disputed_at: Option<OffsetDateTime>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RulingView {
    pub to_buyer_bps: Option<i32>,
    pub bond_to_buyer_bps: Option<i32>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SettlementView {
    pub to_buyer: Option<String>,
    pub to_seller: Option<String>,
    pub to_fee_recipient: Option<String>,
}

/// 订单的来源：卖家签名的报价（站内卡片 `GET /api/v1/offers/{id}`）与交易帖内容哈希（下单时锁定的版本）。
#[derive(Debug, Serialize, ToSchema)]
pub struct OrderSource {
    pub offer_hash: String,
    /// 站内报价卡片；拍卖成交或站外签出的报价没有。
    pub offer_id: Option<i64>,
    pub listing_hash: String,
    pub listing_id: Option<i64>,
    pub listing_topic_id: Option<i64>,
    /// 拍卖成交的订单。
    pub auction: Option<AuctionView>,
}

/// 拍卖成交（ADR 0011）：付款页显示拍卖币种成交价、卖家汇率与应付 USDT（订单 `amount`）。
#[derive(Debug, Serialize, ToSchema)]
pub struct AuctionView {
    pub auction_id: String,
    /// `usdt` 或 `cny`。
    pub currency: &'static str,
    /// 卖家签名的固定汇率（每 1 USDT 合多少元，4 位小数定点数，7.2000 = 72000）；USDT 拍卖为空。
    pub cny_per_usdt: Option<i32>,
    /// 成交价，拍卖币种的最小单位：USDT 6 位小数或人民币分（十进制字符串）。
    pub price: String,
}

impl AuctionView {
    fn from_row(
        auction_id: Option<String>,
        currency: Option<i16>,
        cny_per_usdt: Option<i32>,
        price: Option<String>,
    ) -> Option<Self> {
        let (auction_id, price) = auction_id.zip(price)?;
        let currency = match currency? {
            0 => "usdt",
            1 => "cny",
            _ => return None,
        };
        Some(Self {
            auction_id,
            currency,
            cny_per_usdt: cny_per_usdt.filter(|r| *r != 0),
            price,
        })
    }
}

/// 私信报价生成的订单：谈判会话与付款上链时的最后一条消息（`GET /api/v1/orders/{id}/negotiation` 读取）。
#[derive(Debug, Serialize, ToSchema)]
pub struct NegotiationRef {
    pub conversation_id: i64,
    pub last_message_id: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OrderView {
    pub id: i64,
    pub chain_id: i64,
    pub contract: String,
    pub chain_order_id: i64,
    /// `escrow`（一次性）或 `stream`（分期）。
    pub mode: &'static str,
    /// `funded` / `disputed` / `released` / `refunded` / `settled`；`none` = 下单交易已被区块回滚撤销。
    pub state: &'static str,
    /// 确认级别：`latest` / `safe` / `finalized`。
    pub level: String,
    pub buyer: Party,
    pub seller: Party,
    pub token: String,
    pub amount: String,
    pub fees: OrderFees,
    pub instant: bool,
    pub deadlines: Deadlines,
    pub stream: Option<StreamView>,
    pub proposal: Option<ProposalView>,
    pub dispute: Option<DisputeView>,
    pub ruling: Option<RulingView>,
    pub settlement: Option<SettlementView>,
    pub source: OrderSource,
    /// 建单前的私信谈判记录（仲裁证据）。
    pub negotiation: Option<NegotiationRef>,
    pub created_block: i64,
    pub updated_block: i64,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

impl OrderView {
    pub(crate) fn of(r: OrderRow) -> ApiResult<Self> {
        let market = Market::from_mode(r.mode).map_err(ApiError::internal)?;
        let state = u8::try_from(r.state)
            .map_err(ApiError::internal)
            .and_then(|s| OrderState::from_u8(s).map_err(ApiError::internal))?;
        let stream = match (market, r.interval_secs, r.periods) {
            (Market::Stream, Some(interval_secs), Some(periods)) => Some(StreamView {
                interval_secs,
                periods,
                started_at: r.started_at,
                ends_at: r
                    .started_at
                    .map(|t| t + Duration::seconds(i64::from(interval_secs) * i64::from(periods))),
                claimed: r.released_amount,
            }),
            _ => None,
        };
        // 分期订单的 ship_deadline 列存的是开始期限。
        let (ship, start) = match market {
            Market::Escrow => (r.ship_deadline, None),
            Market::Stream => (None, r.ship_deadline),
        };
        Ok(Self {
            id: r.id,
            chain_id: r.chain_id,
            contract: hex0x(&r.contract),
            chain_order_id: r.chain_order_id,
            mode: market.as_str(),
            state: state.as_str(),
            level: r.level,
            buyer: Party {
                address: hex0x(&r.buyer),
                user_id: r.buyer_id,
            },
            seller: Party {
                address: hex0x(&r.seller),
                user_id: r.seller_id,
            },
            token: hex0x(&r.token),
            amount: r.amount,
            fees: OrderFees {
                buyer_fee: r.buyer_fee,
                seller_fee: r.seller_fee,
                buyer_gas: r.buyer_gas,
                seller_gas: r.seller_gas,
                dispute_fee: r.dispute_fee,
                bond: r.bond,
            },
            instant: r.instant,
            deadlines: Deadlines {
                ship,
                confirm: r.confirm_deadline,
                start,
            },
            stream,
            proposal: r.proposer.map(|p| ProposalView {
                proposer: hex0x(&p),
                to_buyer: r.proposal_to_buyer.unwrap_or_default(),
                expiry: r.proposal_expiry,
            }),
            dispute: r.disputed_by.map(|by| DisputeView {
                by: hex0x(&by),
                from_bond: r.dispute_from_bond.unwrap_or(false),
                disputed_at: r.disputed_at,
            }),
            ruling: (r.ruling_to_buyer_bps.is_some() || r.ruling_bond_to_buyer_bps.is_some())
                .then_some(RulingView {
                    to_buyer_bps: r.ruling_to_buyer_bps,
                    bond_to_buyer_bps: r.ruling_bond_to_buyer_bps,
                }),
            settlement: r.settled_to_buyer.is_some().then_some(SettlementView {
                to_buyer: r.settled_to_buyer,
                to_seller: r.settled_to_seller,
                to_fee_recipient: r.settled_to_fee,
            }),
            source: OrderSource {
                offer_hash: hex0x(&r.offer_hash),
                offer_id: r.offer_id,
                listing_hash: hex0x(&r.listing_hash),
                listing_id: r.listing_id,
                listing_topic_id: r.listing_topic_id,
                auction: AuctionView::from_row(
                    r.auction_id,
                    r.auction_currency,
                    r.auction_cny_per_usdt,
                    r.auction_price,
                ),
            },
            negotiation: r
                .negotiation_conversation_id
                .zip(r.negotiation_last_message_id)
                .map(|(conversation_id, last_message_id)| NegotiationRef {
                    conversation_id,
                    last_message_id,
                }),
            created_block: r.created_block,
            updated_block: r.updated_block,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
    }
}

/// 我的订单：按角色分别走 `(buyer_id, id DESC)` / `(seller_id, id DESC)` 索引，新的在前，每页 20 条。
#[utoipa::path(
    get, path = "/api/v1/orders", tag = "orders", operation_id = "list_orders",
    params(OrdersQuery),
    responses((status = 200, body = OrderPage), (status = 400, body = ErrorBody), (status = 401, body = ErrorBody))
)]
pub async fn list(
    State(state): State<AppState>,
    session: Session,
    Query(q): Query<OrdersQuery>,
) -> ApiResult<Json<OrderPage>> {
    let user_id = current_user_id(&session).await?;
    let role = match q.role {
        OrderRole::Buyer => Role::Buyer,
        OrderRole::Seller => Role::Seller,
    };
    let before = q.cursor.unwrap_or(i64::MAX);
    let ids = db::page_ids(&state.db, role, user_id, before, db::ORDERS_PAGE).await?;
    let next_cursor = ids
        .last()
        .copied()
        .filter(|_| i64::try_from(ids.len()) == Ok(db::ORDERS_PAGE));
    let rows = db::by_ids(&state.db, &ids).await?;
    Ok(Json(OrderPage {
        orders: rows
            .into_iter()
            .map(OrderView::of)
            .collect::<ApiResult<_>>()?,
        next_cursor,
    }))
}

/// 订单详情：买卖双方与该订单仲裁案件的仲裁员、站长可见。
#[utoipa::path(
    get, path = "/api/v1/orders/{id}", tag = "orders", operation_id = "get_order",
    params(("id" = i64, Path)),
    responses((status = 200, body = OrderView), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody))
)]
pub async fn get(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
) -> ApiResult<Json<OrderView>> {
    let user_id = current_user_id(&session).await?;
    let row = db::by_ids(&state.db, &[id])
        .await?
        .pop()
        .ok_or(ApiError(ErrorCode::NotFound))?;
    let party = row.buyer_id == Some(user_id) || row.seller_id == Some(user_id);
    if !party && !forum_db::arbitration::can_review_order(&state.db, id, user_id).await? {
        return Err(ApiError(ErrorCode::NotFound));
    }
    Ok(Json(OrderView::of(row)?))
}

/// 报价卡片生成的订单（买家付款后从卡片跳到订单页）：只有该订单的买卖双方能查到；
/// 可多次成交的公开报价返回当前用户参与的最新一单。
#[utoipa::path(
    get, path = "/api/v1/offers/{id}/order", tag = "orders",
    params(("id" = i64, Path)),
    responses((status = 200, body = OrderView), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody))
)]
pub async fn by_offer(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
) -> ApiResult<Json<OrderView>> {
    let user_id = current_user_id(&session).await?;
    let not_found = || ApiError(ErrorCode::NotFound);
    let offer = forum_db::trade::offer(&state.db, id)
        .await?
        .ok_or_else(not_found)?;
    let order_id = db::id_by_offer(
        &state.db,
        &offer.offer_hash,
        offer.chain_id,
        &offer.verifying_contract,
        user_id,
    )
    .await?
    .ok_or_else(not_found)?;
    let row = db::by_ids(&state.db, &[order_id])
        .await?
        .pop()
        .ok_or_else(not_found)?;
    Ok(Json(OrderView::of(row)?))
}
