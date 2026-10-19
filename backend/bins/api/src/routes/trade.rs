//! 交易帖与担保约定卡片（plan 第 6、7、9 节）。
//!
//! - 交易帖 = 普通主题（kind = 交易）+ `trade_listings` 一行，同一事务写入。
//! - 报价卡片 = 卖家签名的 EIP-712 `Offer`。服务端用 Rust 重算 `hashOffer`（与合约逐字节一致），
//!   并在签名前就按合约常量校验条款，界面能提前失败。
//! - 签名本身暂不在服务端验证：卖家是智能账户（可能未部署，走 ERC-6492），需要链上 RPC。
//!   签名无效的报价只会在付款前的模拟里失败（forum-sponsor 签字前必模拟，不会白扣代付费）。
//! - 卡片状态：待确认 →（卖家修改）已修改 / （买家）已拒绝 / （卖家）已取消 / （worker）已过期 / （indexer）已付款。
//!   只有待确认且未过期的卡片能变，状态变化是一条带条件的 UPDATE，并发下只会成功一次。

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use forum_chain::config::WalletInitVersion;
use forum_core::eip712::{Address, B256, DOMAIN_NAME, DOMAIN_VERSION, Offer, U256, keccak256};
use forum_core::trade::{
    MAX_OFFER_TTL_SECS, MAX_WINDOW, MIN_OFFER_TTL_SECS, MIN_WINDOW, check_bond, check_offer,
    check_windows,
};
use forum_core::{ErrorCode, FeeParams, FeePayer, Usdt};
use forum_db::trade::{self as db, NewListing, NewOffer, OfferRow, OfferStatus};
use forum_db::{topics, wallet as wallets};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tower_sessions::Session;
use utoipa::ToSchema;

use super::forums::{NewTopicRequest, prepare_topic};
use super::me::current_user_id;
use crate::access::Viewer;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::state::AppState;
use crate::views::hex0x;
use crate::wallet::recent_passkey;

/// 交易帖预填（plan 第 9 节「交易帖预填」）：发货 3 天、确认 7 天、卖家承担手续费。
const DEFAULT_SHIP_WINDOW: u32 = 3 * 86_400;
const DEFAULT_CONFIRM_WINDOW: u32 = 7 * 86_400;
/// 公开报价默认 30 天、私信报价默认 24 小时有效。
const DEFAULT_PUBLIC_TTL: u64 = 30 * 86_400;
const DEFAULT_PRIVATE_TTL: u64 = 86_400;
/// 签名最大字节数：ERC-6492 包装（工厂地址 + initCode + 内层签名）也远小于此。
const MAX_SIGNATURE_BYTES: usize = 4096;
const MAX_REASON_CHARS: usize = 200;

fn fee_payer_of(code: u8) -> ApiResult<FeePayer> {
    Ok(match code {
        0 => FeePayer::Seller,
        1 => FeePayer::Buyer,
        2 => FeePayer::Half,
        _ => return Err(ApiError(ErrorCode::InvalidParameter)),
    })
}

fn usdt_of(s: &str) -> ApiResult<Usdt> {
    let v: U256 = s
        .parse()
        .map_err(|_| ApiError(ErrorCode::InvalidParameter))?;
    v.to_u128()
        .map(Usdt)
        .ok_or(ApiError(ErrorCode::AmountOutOfRange))
}

// ───────────────────────────── 参数 ─────────────────────────────

#[derive(Debug, Serialize, ToSchema)]
pub struct Eip712Domain {
    pub name: &'static str,
    pub version: &'static str,
    #[serde(rename = "chainId")]
    pub chain_id: u64,
    /// 担保合约地址；未配置时为空，报价功能不可用。
    #[serde(rename = "verifyingContract")]
    pub verifying_contract: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TradeParams {
    pub domain: Eip712Domain,
    /// 本链 USDT 地址（报价的 `token`）。
    pub token: Option<String>,
    /// 以下金额均为 USDT 最小单位（十进制字符串）。
    pub min_order: String,
    pub order_cap: String,
    /// 当前收费：报价里的 `maxFeeBps` / `maxBaseGas` / `maxMinFee` 不能低于这些值。
    pub fee_bps: u16,
    pub min_fee: String,
    pub base_gas: String,
    pub min_window: u32,
    pub max_window: u32,
    pub min_offer_ttl: u64,
    pub max_offer_ttl: u64,
    pub defaults: TradeDefaults,
    /// 钱包初始化参数各版本（ADR 0008），按版本号升序；新钱包用最后一个，已有钱包用自己登记的版本。
    pub wallet_init_versions: Vec<WalletInitVersionView>,
}

/// 一个版本的 Kernel v3.3 账户初始化参数。地址为 `0x` 小写十六进制。
#[derive(Debug, Serialize, ToSchema)]
pub struct WalletInitVersionView {
    pub version: u32,
    pub kernel_implementation: String,
    pub kernel_factory: String,
    pub kernel_meta_factory: String,
    pub webauthn_validator: String,
    pub entry_point: String,
    pub paymaster: String,
    pub approve_token: String,
    /// 十进制字符串。
    pub approve_amount: String,
}

impl From<&WalletInitVersion> for WalletInitVersionView {
    fn from(v: &WalletInitVersion) -> Self {
        Self {
            version: v.version,
            kernel_implementation: hex0x(v.kernel_implementation.as_slice()),
            kernel_factory: hex0x(v.kernel_factory.as_slice()),
            kernel_meta_factory: hex0x(v.kernel_meta_factory.as_slice()),
            webauthn_validator: hex0x(v.webauthn_validator.as_slice()),
            entry_point: hex0x(v.entry_point.as_slice()),
            paymaster: hex0x(v.paymaster.as_slice()),
            approve_token: hex0x(v.approve_token.as_slice()),
            approve_amount: v.approve_amount.to_string(),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TradeDefaults {
    pub ship_window: u32,
    pub confirm_window: u32,
    /// 0 = 卖家承担（默认）。
    pub fee_payer: u8,
    pub public_offer_ttl: u64,
    pub private_offer_ttl: u64,
}

/// 报价签名域、链上范围与预填值。前端在签名前用它们校验，避免签出链上必然拒绝的报价。
#[utoipa::path(get, path = "/api/v1/trade/params", tag = "trade", responses((status = 200, body = TradeParams)))]
pub async fn params(State(state): State<AppState>) -> Json<TradeParams> {
    let w = &state.wallet;
    let l = &w.limits;
    Json(TradeParams {
        domain: Eip712Domain {
            name: DOMAIN_NAME,
            version: DOMAIN_VERSION,
            chain_id: w.chain.chain_id,
            verifying_contract: w.chain.escrow.map(|a| a.to_string()),
        },
        token: w.chain.usdt.map(|a| a.to_string()),
        min_order: l.min_order.0.to_string(),
        order_cap: l.order_cap.0.to_string(),
        fee_bps: l.fee_bps,
        min_fee: l.min_fee.0.to_string(),
        base_gas: l.base_gas.0.to_string(),
        min_window: MIN_WINDOW,
        max_window: MAX_WINDOW,
        min_offer_ttl: MIN_OFFER_TTL_SECS,
        max_offer_ttl: MAX_OFFER_TTL_SECS,
        defaults: TradeDefaults {
            ship_window: DEFAULT_SHIP_WINDOW,
            confirm_window: DEFAULT_CONFIRM_WINDOW,
            fee_payer: 0,
            public_offer_ttl: DEFAULT_PUBLIC_TTL,
            private_offer_ttl: DEFAULT_PRIVATE_TTL,
        },
        wallet_init_versions: w
            .chain
            .wallet_init_versions
            .iter()
            .map(WalletInitVersionView::from)
            .collect(),
    })
}

// ───────────────────────────── 交易帖 ─────────────────────────────

/// 交易帖条款（预填给报价；实际成交以卖家签名的报价为准）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ListingTerms {
    #[serde(default)]
    pub instant: bool,
    pub ship_window: u32,
    pub confirm_window: u32,
    /// 0 = 卖家、1 = 买家、2 = 各半。
    #[serde(default)]
    pub fee_payer: u8,
    /// 卖家保证金（最小单位，十进制字符串）；默认 "0"。
    #[serde(default = "zero")]
    pub bond: String,
    #[serde(default)]
    pub late_slash_bps: u16,
}

fn zero() -> String {
    "0".to_owned()
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewListingRequest {
    #[serde(flatten)]
    pub topic: NewTopicRequest,
    /// 价格，USDT 最小单位（十进制字符串，如 "45000000" = 45 USDT）。
    pub price: String,
    pub terms: ListingTerms,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreatedListing {
    pub listing_id: i64,
    pub topic_id: i64,
    pub post_id: i64,
    /// 报价里的 `listingHash`（`0x` + 64 位十六进制）。
    pub content_hash: String,
}

/// 交易帖内容哈希的原文：版本号 + 标题 + 过滤后的 HTML + 价格 + 条款，按字段固定顺序序列化成 JSON 后取 keccak256。
/// 下单时论坛保存同一份快照，之后编辑帖子不影响已签报价（plan 第 6 节「交易帖内容锁定」）。
#[derive(Serialize)]
struct ListingSnapshot<'a> {
    v: u8,
    title: &'a str,
    html: &'a str,
    price: &'a str,
    terms: &'a ListingTerms,
}

/// 发交易帖：主题（kind = 交易）+ 交易条款同一事务写入。需要已开通交易（本链有钱包）。
#[utoipa::path(
    post, path = "/api/v1/forums/{slug}/listings", tag = "trade",
    params(("slug" = String, Path)),
    request_body = NewListingRequest,
    responses(
        (status = 201, body = CreatedListing),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody), (status = 429, body = ErrorBody)
    )
)]
pub async fn create_listing(
    State(state): State<AppState>,
    session: Session,
    Path(slug): Path<String>,
    Json(req): Json<NewListingRequest>,
) -> ApiResult<(StatusCode, Json<CreatedListing>)> {
    let user_id = current_user_id(&session).await?;
    let chain_id = state.wallet.chain_id();
    if wallets::wallet_on(&state.db, user_id, chain_id)
        .await?
        .is_none()
    {
        return Err(ApiError(ErrorCode::WalletRequired));
    }
    let price = usdt_of(&req.price)?;
    state.wallet.limits.check_amount(price)?;
    let t = &req.terms;
    fee_payer_of(t.fee_payer)?;
    check_windows(t.instant, t.ship_window, t.confirm_window)?;
    let bond = usdt_of(&t.bond)?;
    check_bond(price, bond, t.late_slash_bps)?;
    // 规范化数字字符串（去掉前导 0），哈希与数据库里的值一致。
    let price_str = price.0.to_string();
    let terms = ListingTerms {
        bond: bond.0.to_string(),
        ..t.clone()
    };

    let p = prepare_topic(&state, &session, &slug, &req.topic).await?;
    let snapshot = ListingSnapshot {
        v: 1,
        title: &p.title,
        html: &p.cleaned.html,
        price: &price_str,
        terms: &terms,
    };
    let content_hash = keccak256(&serde_json::to_vec(&snapshot).map_err(ApiError::internal)?);
    let terms_json = serde_json::to_value(&terms).map_err(ApiError::internal)?;

    let mut tx = state.db.begin().await?;
    let created = topics::create(&mut *tx, p.new_topic(&req.topic, topics::KIND_TRADE)).await?;
    let listing_id = db::insert_listing(
        &mut *tx,
        NewListing {
            topic_id: created.topic_id,
            seller_id: p.actor.id,
            chain_id,
            price: &price_str,
            terms: &terms_json,
            content_hash: &content_hash,
        },
    )
    .await?;
    tx.commit().await?;
    p.after_create(&state, &created).await;
    Ok((
        StatusCode::CREATED,
        Json(CreatedListing {
            listing_id,
            topic_id: created.topic_id,
            post_id: created.post_id,
            content_hash: hex0x(&content_hash),
        }),
    ))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ListingView {
    pub id: i64,
    pub topic_id: i64,
    pub seller_id: i64,
    pub chain_id: i64,
    pub price: String,
    pub terms: ListingTerms,
    pub content_hash: String,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// 读交易帖，并按所在主题的可见性检查（看不到主题就看不到交易帖）。
async fn visible_listing(
    state: &AppState,
    session: &Session,
    id: i64,
) -> ApiResult<(db::Listing, Viewer)> {
    let not_found = || ApiError(ErrorCode::NotFound);
    let listing = db::listing(&state.db, id).await?.ok_or_else(not_found)?;
    let topic = topics::by_id(&state.db, listing.topic_id)
        .await?
        .ok_or_else(not_found)?;
    let forum = forum_db::forums::by_id(&state.db, topic.forum_id)
        .await?
        .ok_or_else(not_found)?;
    let viewer = Viewer::load(state, session).await?;
    let hidden = topic.flags & topics::FLAG_HIDDEN != 0;
    if (hidden || viewer.level() < forum.view_min_level)
        && !viewer.is_staff(state, forum.id).await?
    {
        return Err(not_found());
    }
    Ok((listing, viewer))
}

/// 交易帖详情（价格、条款、内容哈希）。
#[utoipa::path(
    get, path = "/api/v1/listings/{id}", tag = "trade",
    params(("id" = i64, Path)),
    responses((status = 200, body = ListingView), (status = 404, body = ErrorBody))
)]
pub async fn get_listing(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
) -> ApiResult<Json<ListingView>> {
    let (l, _) = visible_listing(&state, &session, id).await?;
    Ok(Json(ListingView {
        id: l.id,
        topic_id: l.topic_id,
        seller_id: l.seller_id,
        chain_id: l.chain_id,
        price: l.price,
        terms: serde_json::from_value(l.terms).map_err(ApiError::internal)?,
        content_hash: hex0x(&l.content_hash),
        created_at: l.created_at,
    }))
}

// ───────────────────────────── 报价卡片 ─────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum CardStatus {
    Pending,
    Modified,
    Rejected,
    Cancelled,
    Expired,
    Paid,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OfferView {
    pub id: i64,
    pub listing_id: Option<i64>,
    pub conversation_id: Option<i64>,
    pub seller_id: i64,
    pub buyer_id: Option<i64>,
    /// 被这张卡片替换的旧卡片（修改约定）。
    pub replaces_id: Option<i64>,
    pub status: CardStatus,
    pub reject_reason: Option<String>,
    pub chain_id: i64,
    pub verifying_contract: String,
    /// EIP-712 typed data 的 `message`。
    pub offer: Offer,
    pub signature: String,
    pub offer_hash: String,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

fn bytes<const N: usize>(b: &[u8]) -> ApiResult<[u8; N]> {
    b.try_into().map_err(ApiError::internal)
}

fn num<T: TryFrom<i64>>(v: i64) -> ApiResult<T> {
    T::try_from(v).map_err(|_| ApiError::internal("offer column out of range"))
}

fn u256(s: &str) -> ApiResult<U256> {
    s.parse().map_err(ApiError::internal)
}

/// 数据库行还原成签名时的 `Offer`。
fn offer_of(r: &OfferRow) -> ApiResult<Offer> {
    Ok(Offer {
        seller: Address(bytes(&r.seller)?),
        buyer: Address(bytes(&r.buyer)?),
        token: Address(bytes(&r.token)?),
        amount: u256(&r.amount)?,
        fee_payer: num(r.fee_payer.into())?,
        max_fee_bps: num(r.max_fee_bps.into())?,
        max_base_gas: u256(&r.max_base_gas)?,
        max_min_fee: u256(&r.max_min_fee)?,
        bond: u256(&r.bond)?,
        late_slash_bps: num(r.late_slash_bps.into())?,
        instant: r.instant,
        ship_window: num(r.ship_window)?,
        confirm_window: num(r.confirm_window)?,
        listing_hash: B256(bytes(&r.listing_hash)?),
        expiry: num(r.expiry)?,
        max_fills: num(r.max_fills)?,
        nonce: u256(&r.nonce)?,
    })
}

/// 待确认但已过有效期（worker 还没来得及改）的卡片按已过期展示。
fn card_status(r: &OfferRow) -> CardStatus {
    match r.status {
        OfferStatus::Pending if r.expires_at <= OffsetDateTime::now_utc() => CardStatus::Expired,
        OfferStatus::Pending => CardStatus::Pending,
        OfferStatus::Modified => CardStatus::Modified,
        OfferStatus::Rejected => CardStatus::Rejected,
        OfferStatus::Cancelled => CardStatus::Cancelled,
        OfferStatus::Expired => CardStatus::Expired,
        OfferStatus::Paid => CardStatus::Paid,
    }
}

impl OfferView {
    fn of(r: &OfferRow) -> ApiResult<Self> {
        Ok(Self {
            id: r.id,
            listing_id: r.listing_id,
            conversation_id: r.conversation_id,
            seller_id: r.seller_id,
            buyer_id: r.buyer_id,
            replaces_id: r.replaces_id,
            status: card_status(r),
            reject_reason: r.reject_reason.clone(),
            chain_id: r.chain_id,
            verifying_contract: hex0x(&r.verifying_contract),
            offer: offer_of(r)?,
            signature: hex0x(&r.signature),
            offer_hash: hex0x(&r.offer_hash),
            expires_at: r.expires_at,
            created_at: r.created_at,
        })
    }
}

fn views(rows: &[OfferRow]) -> ApiResult<Vec<OfferView>> {
    rows.iter().map(OfferView::of).collect()
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NewOfferRequest {
    /// 交易帖下的报价：`offer.listingHash` 必须等于该帖的 `content_hash`。
    pub listing_id: Option<i64>,
    /// 私信里的报价：必须指定买家、只能成交 1 次。
    pub conversation_id: Option<i64>,
    /// 修改约定：被替换的待确认卡片（同一交易帖 / 会话、同一卖家），旧卡片变为「已修改」。
    pub replaces: Option<i64>,
    pub offer: Offer,
    /// 卖家对 `hashOffer(offer)` 的签名（`0x` 十六进制；智能账户为 ERC-1271 / ERC-6492 格式）。
    pub signature: String,
}

fn parse_signature(s: &str) -> ApiResult<Vec<u8>> {
    let bad = || ApiError(ErrorCode::InvalidParameter);
    let sig = hex::decode(s.strip_prefix("0x").ok_or_else(bad)?).map_err(|_| bad())?;
    if sig.is_empty() || sig.len() > MAX_SIGNATURE_BYTES {
        return Err(bad());
    }
    Ok(sig)
}

/// 卖家提交签好的报价卡片（需 5 分钟内的钱包 Passkey 验证：plan「私信发起担保」是敏感操作）。
#[utoipa::path(
    post, path = "/api/v1/offers", tag = "trade",
    request_body = NewOfferRequest,
    responses(
        (status = 201, body = OfferView),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody), (status = 409, body = ErrorBody)
    )
)]
pub async fn create_offer(
    State(state): State<AppState>,
    session: Session,
    Json(req): Json<NewOfferRequest>,
) -> ApiResult<(StatusCode, Json<OfferView>)> {
    let user_id = current_user_id(&session).await?;
    recent_passkey(&session).await?;
    let w = &state.wallet;
    let domain = w.domain()?;
    let token = w.usdt()?;
    let chain_id = w.chain_id();
    let o = &req.offer;

    let seller_wallet = wallets::wallet_on(&state.db, user_id, chain_id)
        .await?
        .ok_or(ApiError(ErrorCode::WalletRequired))?;
    if seller_wallet.address != o.seller.0 {
        return Err(ApiError(ErrorCode::Forbidden));
    }
    let now = u64::try_from(OffsetDateTime::now_utc().unix_timestamp()).unwrap_or(0);
    check_offer(o, &w.limits, &token, now)?;
    let signature = parse_signature(&req.signature)?;

    match (req.listing_id, req.conversation_id) {
        (None, None) => return Err(ApiError(ErrorCode::InvalidParameter)),
        (_, Some(_)) if o.buyer.is_zero() || o.max_fills != 1 => {
            return Err(ApiError(ErrorCode::InvalidParameter));
        }
        _ => {}
    }
    if let Some(listing_id) = req.listing_id {
        let listing = db::listing(&state.db, listing_id)
            .await?
            .ok_or(ApiError(ErrorCode::NotFound))?;
        if listing.seller_id != user_id {
            return Err(ApiError(ErrorCode::Forbidden));
        }
        if listing.chain_id != chain_id || listing.content_hash != o.listing_hash.0 {
            return Err(ApiError(ErrorCode::InvalidParameter));
        }
    }
    // 指定买家必须是已开通交易的用户，且不是卖家自己。
    let buyer_id = if o.buyer.is_zero() {
        None
    } else {
        match wallets::owner_of(&state.db, chain_id, &o.buyer.0).await? {
            Some(id) if id != user_id => Some(id),
            _ => return Err(ApiError(ErrorCode::InvalidParameter)),
        }
    };

    let offer_hash = o.hash(&domain);
    let (amount, max_base_gas, max_min_fee, bond, nonce) = (
        o.amount.to_string(),
        o.max_base_gas.to_string(),
        o.max_min_fee.to_string(),
        o.bond.to_string(),
        o.nonce.to_string(),
    );
    let mut tx = state.db.begin().await?;
    if let Some(old_id) = req.replaces {
        let old = db::offer(&mut *tx, old_id)
            .await?
            .ok_or(ApiError(ErrorCode::NotFound))?;
        if old.seller_id != user_id
            || old.listing_id != req.listing_id
            || old.conversation_id != req.conversation_id
        {
            return Err(ApiError(ErrorCode::Forbidden));
        }
        if !db::close_offer(&mut *tx, old_id, OfferStatus::Modified, None).await? {
            return Err(ApiError(ErrorCode::OfferUnavailable));
        }
    }
    let id = db::insert_offer(
        &mut *tx,
        NewOffer {
            listing_id: req.listing_id,
            conversation_id: req.conversation_id,
            seller_id: user_id,
            buyer_id,
            replaces_id: req.replaces,
            chain_id,
            verifying_contract: &domain.verifying_contract.0,
            seller: &o.seller.0,
            buyer: &o.buyer.0,
            token: &o.token.0,
            amount: &amount,
            fee_payer: o.fee_payer.into(),
            max_fee_bps: o.max_fee_bps.into(),
            max_base_gas: &max_base_gas,
            max_min_fee: &max_min_fee,
            bond: &bond,
            late_slash_bps: o.late_slash_bps.into(),
            instant: o.instant,
            ship_window: o.ship_window.into(),
            confirm_window: o.confirm_window.into(),
            listing_hash: &o.listing_hash.0,
            expiry: i64::try_from(o.expiry).map_err(|_| ApiError(ErrorCode::OfferExpiryInvalid))?,
            max_fills: o.max_fills.into(),
            nonce: &nonce,
            signature: &signature,
            offer_hash: &offer_hash.0,
        },
    )
    .await?
    .ok_or(ApiError(ErrorCode::Conflict))?;
    let row = db::offer(&mut *tx, id)
        .await?
        .ok_or_else(|| ApiError::internal("offer vanished after insert"))?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(OfferView::of(&row)?)))
}

/// 读一张卡片：指定买家的只有买卖双方能看；公开报价跟随所在交易帖的可见性。
async fn visible_offer(state: &AppState, session: &Session, id: i64) -> ApiResult<OfferRow> {
    let viewer = session.get::<i64>(super::SESSION_USER_ID).await?;
    let row = db::offer(&state.db, id)
        .await?
        .ok_or(ApiError(ErrorCode::NotFound))?;
    let party = viewer.is_some_and(|v| v == row.seller_id || Some(v) == row.buyer_id);
    if !party {
        match (row.buyer_id, row.listing_id) {
            (Some(_), _) => return Err(ApiError(ErrorCode::NotFound)),
            (None, Some(listing_id)) => {
                visible_listing(state, session, listing_id).await?;
            }
            (None, None) => {}
        }
    }
    Ok(row)
}

#[utoipa::path(
    get, path = "/api/v1/offers/{id}", tag = "trade",
    params(("id" = i64, Path)),
    responses((status = 200, body = OfferView), (status = 404, body = ErrorBody))
)]
pub async fn get_offer(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
) -> ApiResult<Json<OfferView>> {
    let row = visible_offer(&state, &session, id).await?;
    Ok(Json(OfferView::of(&row)?))
}

/// 交易帖下的报价卡片（新的在前，最多 50 张）。
#[utoipa::path(
    get, path = "/api/v1/listings/{id}/offers", tag = "trade",
    params(("id" = i64, Path)),
    responses((status = 200, body = Vec<OfferView>), (status = 404, body = ErrorBody))
)]
pub async fn listing_offers(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
) -> ApiResult<Json<Vec<OfferView>>> {
    let (listing, viewer) = visible_listing(&state, &session, id).await?;
    let rows = db::listing_offers(&state.db, listing.id, viewer.user.map(|u| u.id)).await?;
    Ok(Json(views(&rows)?))
}

/// 私信会话里的报价卡片（只有买卖双方能看到；新的在前，最多 50 张）。
#[utoipa::path(
    get, path = "/api/v1/conversations/{id}/offers", tag = "trade",
    params(("id" = i64, Path)),
    responses((status = 200, body = Vec<OfferView>), (status = 401, body = ErrorBody))
)]
pub async fn conversation_offers(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
) -> ApiResult<Json<Vec<OfferView>>> {
    let user_id = current_user_id(&session).await?;
    let rows = db::conversation_offers(&state.db, id, user_id).await?;
    Ok(Json(views(&rows)?))
}

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct RejectRequest {
    /// 拒绝理由（可选，≤ 200 字）。
    pub reason: Option<String>,
}

/// 指定买家拒绝卡片。
#[utoipa::path(
    post, path = "/api/v1/offers/{id}/reject", tag = "trade",
    params(("id" = i64, Path)),
    request_body = RejectRequest,
    responses(
        (status = 200, body = OfferView),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody), (status = 409, body = ErrorBody)
    )
)]
pub async fn reject_offer(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
    Json(req): Json<RejectRequest>,
) -> ApiResult<Json<OfferView>> {
    let user_id = current_user_id(&session).await?;
    let reason = req
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty());
    if reason.is_some_and(|r| r.chars().count() > MAX_REASON_CHARS) {
        return Err(ApiError(ErrorCode::InvalidParameter));
    }
    transition(&state, &session, id, OfferStatus::Rejected, reason, |row| {
        row.buyer_id == Some(user_id)
    })
    .await
}

/// 卖家在对方确认前取消卡片。只作废站内卡片（forum-sponsor 不再代付）；签名在有效期内仍可被链上接受，
/// 要立即在链上作废需另调 `cancelOffer(nonce)`（收代付费）。
#[utoipa::path(
    post, path = "/api/v1/offers/{id}/cancel", tag = "trade",
    params(("id" = i64, Path)),
    responses(
        (status = 200, body = OfferView),
        (status = 401, body = ErrorBody), (status = 403, body = ErrorBody), (status = 404, body = ErrorBody),
        (status = 409, body = ErrorBody)
    )
)]
pub async fn cancel_offer(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
) -> ApiResult<Json<OfferView>> {
    let user_id = current_user_id(&session).await?;
    transition(&state, &session, id, OfferStatus::Cancelled, None, |row| {
        row.seller_id == user_id
    })
    .await
}

/// 关闭一张待确认卡片：`allowed` 判断当前用户能否这样做。
async fn transition(
    state: &AppState,
    session: &Session,
    id: i64,
    to: OfferStatus,
    reason: Option<&str>,
    allowed: impl FnOnce(&OfferRow) -> bool,
) -> ApiResult<Json<OfferView>> {
    let row = visible_offer(state, session, id).await?;
    if !allowed(&row) {
        return Err(ApiError(ErrorCode::Forbidden));
    }
    if !db::close_offer(&state.db, id, to, reason).await? {
        return Err(ApiError(ErrorCode::OfferUnavailable));
    }
    let row = db::offer(&state.db, id)
        .await?
        .ok_or(ApiError(ErrorCode::NotFound))?;
    Ok(Json(OfferView::of(&row)?))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PaymentData {
    pub offer_id: i64,
    /// `createAndFund(offer, sellerSig)` 的合约地址与链。
    pub escrow: String,
    pub chain_id: i64,
    pub offer: Offer,
    pub signature: String,
    pub offer_hash: String,
    /// 按当前收费估算（最小单位）：买家需 `approve` 并转入的总额 = 金额 + 买家手续费 + 买家 gas 费。
    /// 以链上建单时的参数为准；免费活动期间实际更少。
    pub buyer_pays: String,
    pub buyer_fee: String,
    pub buyer_gas: String,
    /// 买家智能账户地址（付款必须从这个地址发起）。
    pub buyer_wallet: String,
}

/// 买家接受卡片：返回付款需要的数据。状态不变——买家付款上链即确认，indexer 看到建单事件后标为已付款。
#[utoipa::path(
    post, path = "/api/v1/offers/{id}/accept", tag = "trade",
    params(("id" = i64, Path)),
    responses(
        (status = 200, body = PaymentData),
        (status = 401, body = ErrorBody), (status = 403, body = ErrorBody), (status = 404, body = ErrorBody),
        (status = 409, body = ErrorBody)
    )
)]
pub async fn accept_offer(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
) -> ApiResult<Json<PaymentData>> {
    let user_id = current_user_id(&session).await?;
    let row = visible_offer(&state, &session, id).await?;
    if row.seller_id == user_id || row.buyer_id.is_some_and(|b| b != user_id) {
        return Err(ApiError(ErrorCode::Forbidden));
    }
    if card_status(&row) != CardStatus::Pending {
        return Err(ApiError(ErrorCode::OfferUnavailable));
    }
    let wallet = wallets::wallet_on(&state.db, user_id, row.chain_id)
        .await?
        .ok_or(ApiError(ErrorCode::WalletRequired))?;
    let offer = offer_of(&row)?;
    let amount = offer
        .amount
        .to_u128()
        .map(Usdt)
        .ok_or_else(|| ApiError::internal("stored amount exceeds u128"))?;
    let l = &state.wallet.limits;
    let quote = forum_core::quote(
        amount,
        fee_payer_of(offer.fee_payer)?,
        &FeeParams {
            fee_bps: l.fee_bps.into(),
            min_fee: l.min_fee,
            campaign: false,
            base_gas: l.base_gas,
            // 建单不收争议费，这里用不到。
            dispute_mul_bps: 0,
        },
    )?;
    Ok(Json(PaymentData {
        offer_id: row.id,
        escrow: hex0x(&row.verifying_contract),
        chain_id: row.chain_id,
        offer,
        signature: hex0x(&row.signature),
        offer_hash: hex0x(&row.offer_hash),
        buyer_pays: quote.buyer_pays.0.to_string(),
        buyer_fee: quote.buyer_fee.0.to_string(),
        buyer_gas: quote.buyer_gas.0.to_string(),
        buyer_wallet: hex0x(&wallet.address),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fee_payer_codes_follow_fee_math() {
        // FeeMath.Payer：0 = Seller、1 = Buyer、2 = Half
        assert!(matches!(fee_payer_of(0), Ok(FeePayer::Seller)));
        assert!(matches!(fee_payer_of(1), Ok(FeePayer::Buyer)));
        assert!(matches!(fee_payer_of(2), Ok(FeePayer::Half)));
        assert!(fee_payer_of(3).is_err());
    }

    #[test]
    fn signature_hex() {
        assert!(parse_signature("0x01").is_ok());
        assert!(parse_signature("01").is_err());
        assert!(parse_signature("0x").is_err());
        assert!(parse_signature("0xzz").is_err());
        assert!(parse_signature(&format!("0x{}", "00".repeat(MAX_SIGNATURE_BYTES + 1))).is_err());
    }
}
