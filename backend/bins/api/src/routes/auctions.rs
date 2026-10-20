//! 拍卖（plan 第 6、7、9 节「拍卖」，ADR 0011）：拍卖帖、卖家授权、出价、出价记录、赢家付款数据。
//!
//! - **挂拍**：`POST /forums/{slug}/auctions` 建草稿（帖子内容与条款，还不发帖）→ `POST /auctions/{id}/start`
//!   按当前参数生成 EIP-712 拍卖授权（有效期覆盖时长、顺延上限、付款期与两轮递补）→ 卖家钱包签名 →
//!   `POST /auctions/{id}/finish` 用与合约相同的规则验签（ECDSA / ERC-1271 / ERC-6492），通过后发帖、开始计时。
//! - **出价**：只在论坛里、只追加，最低加价与防狙击顺延由一条 SQL 原子完成（见 `forum_db::auctions`）。卖家本人与
//!   关联账号不能出价；新账号单场有上限；卖家要求余额时读出价人钱包的链上 USDT 余额（缓存几秒）。
//!   被超价的人收到站内通知；订阅了主题页的连接收到 `auction_bid` 推送。
//! - **成交**：结束后由 forum-worker 结算、forum-sponsor 签成交证明；赢家用 `GET /auctions/{id}/payment` 的数据
//!   调用 `createFromAuction`（经 forum-sponsor 代付，只代付这份证明的赢家）。
//! - 拍卖开关（站点设置 `auction.enabled`）关闭时，挂拍、出价、取消一律返回 `AUCTIONS_DISABLED`；已有拍卖照常结算，
//!   赢家照常付款。

use std::collections::BTreeMap;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use forum_chain::ChainError;
use forum_core::auction::{
    AuctionSettings, Currency, ELIGIBILITY_PERCENTS, Eligibility, Pricing, SIGN_WINDOW_SECS,
    min_next_bid,
};
use forum_core::chat::Event;
use forum_core::eip712::{
    AUCTION_TYPE, Address, AuctionAuth, B256, DOMAIN_NAME, DOMAIN_VERSION, SettlementProof, U256,
    keccak256,
};
use forum_core::trade::check_windows;
use forum_core::{ErrorCode, Usdt};
use forum_db::auctions::{self as db, Auction};
use forum_db::{topics, wallet as wallets};
use forum_kv::realtime::{Route, Topic};
use forum_kv::{SlidingWindow, ephemeral};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;
use tower_sessions::Session;
use utoipa::{IntoParams, ToSchema};

use super::forums::{NewTopicRequest, prepare_topic};
use super::me::current_user_id;
use super::trade::{Eip712Domain, buyer_quote, fee_payer_of, parse_signature, usdt_of};
use crate::access::{Viewer, topic_access};
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::guest;
use crate::middleware::ClientIp;
use crate::posting::{check_can_auction, check_sanctions, limit};
use crate::state::AppState;
use crate::views::hex0x;
use crate::wallet::recent_passkey;

/// 出价限频（初始值，按压测结果调整）。
const BID_RATE: SlidingWindow = SlidingWindow::new("auction-bid", 20, Duration::from_secs(60));
/// 出价资格读到的链上余额缓存几秒：连续出价不必每次都读链，余额本来也没有锁定（plan 第 6 节）。
const BALANCE_TTL: Duration = Duration::from_secs(5);
const BIDS_PAGE: i64 = 50;

fn bad() -> ApiError {
    ApiError(ErrorCode::InvalidParameter)
}

fn num(s: &str) -> ApiResult<u128> {
    s.parse().map_err(ApiError::internal)
}

fn unavailable(e: &ChainError) -> ApiError {
    tracing::warn!(error = %e, "auction chain read failed");
    ApiError(ErrorCode::ChainUnavailable)
}

async fn settings(state: &AppState) -> ApiResult<AuctionSettings> {
    Ok(forum_db::settings::auction(&state.db).await?)
}

/// 写操作的入口检查：拍卖开关打开。
async fn enabled(state: &AppState) -> ApiResult<AuctionSettings> {
    let s = settings(state).await?;
    if s.enabled {
        Ok(s)
    } else {
        Err(ApiError(ErrorCode::AuctionsDisabled))
    }
}

fn address(b: &[u8]) -> ApiResult<Address> {
    Ok(Address(b.try_into().map_err(ApiError::internal)?))
}

fn pricing_of(a: &Auction) -> ApiResult<Pricing> {
    let currency = Currency::from_code(a.currency).ok_or_else(|| ApiError::internal("currency"))?;
    let rate = u32::try_from(a.cny_per_usdt).map_err(ApiError::internal)?;
    Ok(Pricing::new(currency, (rate != 0).then_some(rate))?)
}

fn eligibility_of(a: &Auction) -> ApiResult<Eligibility> {
    Eligibility::from_db(a.eligibility, a.eligibility_value.as_deref())
        .ok_or_else(|| ApiError::internal("eligibility"))
}

/// 成交后的担保条款（与 `AuctionAuth` 里的同名字段一致）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AuctionTerms {
    #[serde(default)]
    pub instant: bool,
    pub ship_window: u32,
    pub confirm_window: u32,
    /// 0 = 卖家、1 = 买家、2 = 各半。
    #[serde(default)]
    pub fee_payer: u8,
}

fn terms_of(a: &Auction) -> ApiResult<AuctionTerms> {
    serde_json::from_value(a.terms.clone()).map_err(ApiError::internal)
}

// ───────────────────────────── 参数 ─────────────────────────────

#[derive(Debug, Serialize, ToSchema)]
pub struct AuctionParams {
    /// 站长设置的拍卖参数（含开关）。
    pub settings: AuctionSettings,
    /// 卖家可选的余额比例档位（%）。
    pub eligibility_percents: Vec<u8>,
    /// 开始签名后多少秒内必须提交签名。
    pub sign_window_secs: u32,
    /// 订单金额上下限（USDT 最小单位）：起拍价折合 USDT 不能低于下限，出价折合 USDT 不能超过上限。
    pub min_order: String,
    pub order_cap: String,
}

/// 挂拍表单需要的参数：开关、时长范围、出价资格档位、订单上下限。
#[utoipa::path(get, path = "/api/v1/auctions/params", tag = "auction", responses((status = 200, body = AuctionParams)))]
pub async fn auction_params(State(state): State<AppState>) -> ApiResult<Json<AuctionParams>> {
    let l = &state.wallet.limits;
    Ok(Json(AuctionParams {
        settings: settings(&state).await?,
        eligibility_percents: ELIGIBILITY_PERCENTS.to_vec(),
        sign_window_secs: SIGN_WINDOW_SECS,
        min_order: l.min_order.0.to_string(),
        order_cap: l.order_cap.0.to_string(),
    }))
}

// ───────────────────────────── 视图 ─────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuctionState {
    /// 卖家还没签授权（只有卖家看得到）。
    Draft,
    Live,
    /// 结束时间已到，正在结算。
    Ended,
    /// 已发成交证明，等中标者付款。
    AwaitingPayment,
    Settled,
    /// 流拍：未达保留价、无人出价或递补用完。
    Unsold,
    Cancelled,
}

fn state_of(a: &Auction, now: OffsetDateTime) -> AuctionState {
    AuctionState::of(a.state, a.ends_at, now)
}

impl AuctionState {
    /// 库里的状态加上「结束时间已到、还没结算」的显示状态。
    pub fn of(state: i16, ends_at: Option<OffsetDateTime>, now: OffsetDateTime) -> Self {
        match state {
            db::STATE_DRAFT => Self::Draft,
            db::STATE_LIVE if ends_at.is_some_and(|e| e <= now) => Self::Ended,
            db::STATE_LIVE => Self::Live,
            db::STATE_AWAITING_PAYMENT => Self::AwaitingPayment,
            db::STATE_SETTLED => Self::Settled,
            db::STATE_UNSOLD => Self::Unsold,
            _ => Self::Cancelled,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Live => "live",
            Self::Ended => "ended",
            Self::AwaitingPayment => "awaiting_payment",
            Self::Settled => "settled",
            Self::Unsold => "unsold",
            Self::Cancelled => "cancelled",
        }
    }
}

/// 访问者自己在这场拍卖里的情况。
#[derive(Debug, Serialize, ToSchema)]
pub struct MyAuctionStatus {
    pub seller: bool,
    /// 我的最高出价（拍卖币种最小单位）。
    pub best_bid: Option<String>,
    /// 我是当前最高出价者。
    pub leading: bool,
    /// 我出过价、但已被超过。
    pub outbid: bool,
    /// 当前这份成交证明是给我的（待付款时用 `GET /auctions/{id}/payment` 取付款数据）。
    pub winner: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuctionView {
    pub id: i64,
    /// 上线后的拍卖帖主题。
    pub topic_id: Option<i64>,
    pub forum_id: i32,
    pub seller_id: i64,
    pub state: AuctionState,
    pub currency: Currency,
    /// 人民币拍卖的卖家汇率（每 1 USDT 合多少元的 4 位小数定点数）。
    pub cny_per_usdt: Option<i32>,
    /// 以下金额均为拍卖币种最小单位（USDT 最小单位或分），十进制字符串。
    pub start_price: String,
    pub increment: String,
    /// 保留价只给卖家；其他人只看到是否已达到。
    pub reserve_price: Option<String>,
    pub reserve_met: bool,
    /// 当前价：最高出价，还没有出价时为起拍价。
    pub current_price: String,
    /// 当前价按卖家汇率折合的 USDT（最小单位，向上取整，与合约换算一致）。
    pub current_usdt: String,
    /// 下一口最低出价。
    pub min_next_bid: String,
    pub bid_count: i32,
    pub eligibility: Eligibility,
    pub terms: AuctionTerms,
    #[schema(value_type = Option<String>, format = DateTime)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub starts_at: Option<OffsetDateTime>,
    /// 结束时间（含防狙击顺延）。
    #[schema(value_type = Option<String>, format = DateTime)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub ends_at: Option<OffsetDateTime>,
    /// 拍卖时长（秒，不含顺延）：已顺延 = `ends_at - starts_at - duration_secs`。
    pub duration_secs: i32,
    /// 防狙击：结束前这么多秒内的出价把结束时间顺延 `snipe_extension_secs`，累计不超过 `extension_cap_secs`
    /// （开始签名时取自站长设置，之后不变；草稿还没开始签名时为空）。
    pub snipe_window_secs: Option<i32>,
    pub snipe_extension_secs: Option<i32>,
    pub extension_cap_secs: Option<i32>,
    /// 服务器当前时间：倒计时按它校正本机时钟偏差。
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub server_time: OffsetDateTime,
    /// 已发出的成交证明份数（1 = 赢家，2、3 = 递补）。
    pub round: i16,
    /// 当前一份成交证明的付款截止时间。
    #[schema(value_type = Option<String>, format = DateTime)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub pay_deadline: Option<OffsetDateTime>,
    /// 成交后的站内订单。
    pub order_id: Option<i64>,
    /// 登录用户才有。
    pub my: Option<MyAuctionStatus>,
}

impl AuctionView {
    async fn of(state: &AppState, a: &Auction, viewer: Option<i64>) -> ApiResult<Self> {
        let now = OffsetDateTime::now_utc();
        let pricing = pricing_of(a)?;
        let start = num(&a.start_price)?;
        let top = a.top_amount.as_deref().map(num).transpose()?;
        let current = top.unwrap_or(start);
        let my = match viewer {
            None => None,
            Some(uid) => {
                let best_bid = db::best_bid_of(&state.db, a.id, uid).await?;
                let leading = a.top_bidder_id == Some(uid);
                let winner = if a.state == db::STATE_AWAITING_PAYMENT {
                    db::proof(&state.db, a.id, a.round)
                        .await?
                        .is_some_and(|p| p.bidder_id == uid)
                } else {
                    false
                };
                Some(MyAuctionStatus {
                    seller: a.seller_id == uid,
                    outbid: best_bid.is_some() && !leading,
                    best_bid,
                    leading,
                    winner,
                })
            }
        };
        Ok(Self {
            id: a.id,
            topic_id: a.topic_id,
            forum_id: a.forum_id,
            seller_id: a.seller_id,
            state: state_of(a, now),
            currency: pricing.currency,
            cny_per_usdt: (a.cny_per_usdt != 0).then_some(a.cny_per_usdt),
            start_price: a.start_price.clone(),
            increment: a.increment.clone(),
            reserve_price: (viewer == Some(a.seller_id)).then(|| a.reserve_price.clone()),
            reserve_met: a.reserve_met(),
            current_price: current.to_string(),
            current_usdt: pricing.to_usdt(current)?.0.to_string(),
            min_next_bid: min_next_bid(start, num(&a.increment)?, top, &pricing)?.to_string(),
            bid_count: a.bid_count,
            eligibility: eligibility_of(a)?,
            terms: terms_of(a)?,
            starts_at: a.starts_at,
            ends_at: a.ends_at,
            duration_secs: a.duration_secs,
            snipe_window_secs: a.snipe_window_secs,
            snipe_extension_secs: a.snipe_extension_secs,
            extension_cap_secs: a.extension_cap_secs,
            server_time: now,
            round: a.round,
            pay_deadline: a.pay_deadline,
            order_id: a.order_id,
            my,
        })
    }
}

/// 读拍卖并检查可见性：草稿只有卖家看得到；上线后跟随拍卖帖主题的可见性。
async fn visible(state: &AppState, session: &Session, id: i64) -> ApiResult<(Auction, Viewer)> {
    let not_found = || ApiError(ErrorCode::NotFound);
    let a = db::by_id(&state.db, id).await?.ok_or_else(not_found)?;
    let viewer = Viewer::load(state, session).await?;
    match a.topic_id {
        Some(topic_id) => {
            topic_access(state, &viewer, topic_id).await?;
        }
        None if viewer.user.is_some_and(|u| u.id == a.seller_id) => {}
        None => return Err(not_found()),
    }
    Ok((a, viewer))
}

async fn view(state: &AppState, a: &Auction, viewer: &Viewer) -> ApiResult<Json<AuctionView>> {
    Ok(Json(
        AuctionView::of(state, a, viewer.user.map(|u| u.id)).await?,
    ))
}

#[utoipa::path(
    get, path = "/api/v1/auctions/{id}", tag = "auction",
    params(("id" = i64, Path)),
    responses((status = 200, body = AuctionView), (status = 404, body = ErrorBody))
)]
pub async fn get_auction(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
) -> ApiResult<Json<AuctionView>> {
    let (a, viewer) = visible(&state, &session, id).await?;
    view(&state, &a, &viewer).await
}

/// 拍卖帖主题对应的拍卖（主题页用）。
#[utoipa::path(
    get, path = "/api/v1/topics/{id}/auction", tag = "auction",
    params(("id" = i64, Path)),
    responses((status = 200, body = AuctionView), (status = 404, body = ErrorBody))
)]
pub async fn topic_auction(
    State(state): State<AppState>,
    session: Session,
    Path(topic_id): Path<i64>,
) -> ApiResult<Json<AuctionView>> {
    let id = db::id_by_topic(&state.db, topic_id)
        .await?
        .ok_or(ApiError(ErrorCode::NotFound))?;
    get_auction(State(state), session, Path(id)).await
}

// ───────────────────────────── 挂拍 ─────────────────────────────

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewAuctionRequest {
    #[serde(flatten)]
    pub topic: NewTopicRequest,
    pub currency: Currency,
    /// 人民币拍卖必填：每 1 USDT 合多少元，4 位小数定点数（7.2000 = 72000），范围 50000–100000。
    #[serde(default)]
    pub cny_per_usdt: Option<u32>,
    /// 以下金额为拍卖币种最小单位（USDT 最小单位或分），十进制字符串。
    pub start_price: String,
    /// 加价幅度：不低于 1 USDT（人民币按卖家汇率折算）与起拍价的 1%。
    pub increment: String,
    /// 保留价：不低于起拍价；最高出价达到它才成交。
    pub reserve_price: String,
    /// 拍卖时长（秒），在站长设定的范围内。
    pub duration_secs: u32,
    #[serde(default = "no_eligibility")]
    pub eligibility: Eligibility,
    pub terms: AuctionTerms,
}

fn no_eligibility() -> Eligibility {
    Eligibility::None
}

/// 拍卖帖内容哈希（授权里的 `listingHash`）的原文：版本号 + 标题 + 过滤后的 HTML + 公开的拍卖条件，
/// 按字段固定顺序序列化成 JSON 后取 keccak256。保留价只在授权里，不进帖子。
#[derive(Serialize)]
struct AuctionSnapshot<'a> {
    v: u8,
    kind: &'static str,
    title: &'a str,
    html: &'a str,
    currency: Currency,
    cny_per_usdt: u32,
    start_price: &'a str,
    increment: &'a str,
    duration_secs: u32,
    eligibility: &'a Eligibility,
    terms: &'a AuctionTerms,
}

/// 草稿里保存的帖子内容，上线时原样发帖。
#[derive(Debug, Serialize, Deserialize)]
struct Draft {
    title: String,
    html: String,
    plain_text: String,
    source: Value,
    mentions: Vec<i64>,
}

/// 挂拍第一步：校验条件并保存草稿（还不发帖）。需要已开通交易、没被封号或禁拍，板块允许拍卖帖（`allow_auction`）。
#[utoipa::path(
    post, path = "/api/v1/forums/{slug}/auctions", tag = "auction",
    params(("slug" = String, Path)),
    request_body = NewAuctionRequest,
    responses(
        (status = 201, body = AuctionView),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody), (status = 429, body = ErrorBody)
    )
)]
pub async fn create_auction(
    State(state): State<AppState>,
    session: Session,
    ClientIp(ip): ClientIp,
    Path(slug): Path<String>,
    Json(req): Json<NewAuctionRequest>,
) -> ApiResult<(StatusCode, Json<AuctionView>)> {
    let user_id = current_user_id(&session).await?;
    let settings = enabled(&state).await?;
    let chain_id = state.wallet.chain_id();
    let domain = state.wallet.domain()?;
    let wallet = wallets::wallet_on(&state.db, user_id, chain_id)
        .await?
        .ok_or(ApiError(ErrorCode::WalletRequired))?;

    let pricing = Pricing::new(req.currency, req.cny_per_usdt)?;
    let start = usdt_of(&req.start_price)?.0;
    let increment = usdt_of(&req.increment)?.0;
    let reserve = usdt_of(&req.reserve_price)?.0;
    if start == 0 || reserve < start {
        return Err(bad());
    }
    if increment < pricing.one_usdt().max(start.div_ceil(100)) {
        return Err(ApiError(ErrorCode::AuctionIncrementTooLow));
    }
    // 成交价折合 USDT 必须落在订单上下限内：起拍价不低于下限，保留价不高于上限（否则永远成交不了）。
    let limits = &state.wallet.limits;
    if pricing.to_usdt(start)? < limits.min_order || pricing.to_usdt(reserve)? > limits.order_cap {
        return Err(ApiError(ErrorCode::AmountOutOfRange));
    }
    settings.check_duration(req.duration_secs)?;
    req.eligibility.check(limits.order_cap)?;
    let t = &req.terms;
    fee_payer_of(t.fee_payer)?;
    check_windows(t.instant, t.ship_window, t.confirm_window)?;

    let p = prepare_topic(
        &state,
        &session,
        ip,
        &slug,
        &req.topic,
        topics::KIND_AUCTION,
    )
    .await?;
    check_can_auction(&state, user_id, Some(p.forum_id)).await?;
    // 规范化数字字符串（去掉前导 0），哈希与数据库里的值一致。
    let (start_s, increment_s, reserve_s) = (
        start.to_string(),
        increment.to_string(),
        reserve.to_string(),
    );
    let snapshot = AuctionSnapshot {
        v: 1,
        kind: "auction",
        title: &p.title,
        html: &p.cleaned.html,
        currency: pricing.currency,
        cny_per_usdt: pricing.cny_per_usdt,
        start_price: &start_s,
        increment: &increment_s,
        duration_secs: req.duration_secs,
        eligibility: &req.eligibility,
        terms: t,
    };
    let content_hash = keccak256(&serde_json::to_vec(&snapshot).map_err(ApiError::internal)?);
    let draft = serde_json::to_value(Draft {
        title: p.title.clone(),
        html: p.cleaned.html.clone(),
        plain_text: p.cleaned.plain_text.clone(),
        source: req.topic.source.clone(),
        mentions: p.cleaned.mentions.clone(),
    })
    .map_err(ApiError::internal)?;
    let (eligibility, eligibility_value) = req.eligibility.to_db();
    let terms = serde_json::to_value(t).map_err(ApiError::internal)?;
    let id = db::insert_draft(
        &state.db,
        db::NewAuction {
            seller_id: user_id,
            forum_id: p.forum_id,
            chain_id,
            verifying_contract: &domain.verifying_contract.0,
            seller: &wallet.address,
            currency: i16::from(pricing.currency.code()),
            cny_per_usdt: i32::try_from(pricing.cny_per_usdt).map_err(|_| bad())?,
            start_price: &start_s,
            increment: &increment_s,
            reserve_price: &reserve_s,
            duration_secs: i32::try_from(req.duration_secs).map_err(|_| bad())?,
            eligibility,
            eligibility_value: eligibility_value.as_deref(),
            terms: &terms,
            content_hash: &content_hash,
            draft: &draft,
        },
    )
    .await?;
    let a = db::by_id(&state.db, id)
        .await?
        .ok_or_else(|| ApiError::internal("auction vanished after insert"))?;
    Ok((
        StatusCode::CREATED,
        Json(AuctionView::of(&state, &a, Some(user_id)).await?),
    ))
}

/// 草稿拍卖，且当前用户是卖家。
async fn own_draft(state: &AppState, session: &Session, id: i64) -> ApiResult<(i64, Auction)> {
    let user_id = current_user_id(session).await?;
    let a = db::by_id(&state.db, id)
        .await?
        .filter(|a| a.seller_id == user_id)
        .ok_or(ApiError(ErrorCode::NotFound))?;
    if a.state != db::STATE_DRAFT {
        return Err(ApiError(ErrorCode::AuctionUnavailable));
    }
    Ok((user_id, a))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TypedField {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
}

/// EIP-712 typed data（`signTypedData` 的参数）。`types` 不含 `EIP712Domain`（由 `domain` 推出）。
#[derive(Debug, Serialize, ToSchema)]
pub struct AuctionTypedData {
    pub domain: Eip712Domain,
    pub types: BTreeMap<String, Vec<TypedField>>,
    #[serde(rename = "primaryType")]
    pub primary_type: &'static str,
    pub message: AuctionAuth,
}

/// `Name(type a,type b)` → 字段表。由合约 typehash 的原文生成，与 [`AUCTION_TYPE`] 只有一处定义。
fn typed_fields(encoded: &str) -> (String, Vec<TypedField>) {
    let (name, rest) = encoded.split_once('(').unwrap_or((encoded, ""));
    let fields = rest
        .trim_end_matches(')')
        .split(',')
        .filter_map(|f| f.split_once(' '))
        .map(|(kind, name)| TypedField {
            name: name.to_owned(),
            kind: kind.to_owned(),
        })
        .collect();
    (name.to_owned(), fields)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuctionSigning {
    pub auction_id: i64,
    pub typed_data: AuctionTypedData,
    /// `hashAuction(auth)`：签名的摘要。
    pub auth_hash: String,
    /// 这个时间之前提交签名（`POST /auctions/{id}/finish`），否则重新开始。
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub sign_deadline: OffsetDateTime,
}

/// 挂拍第二步：按当前参数生成拍卖授权，返回给卖家钱包签名的 EIP-712 typed data。
/// 需要 5 分钟内的钱包 Passkey 验证；可以重复调用（换一份新的授权，旧的作废）。
#[utoipa::path(
    post, path = "/api/v1/auctions/{id}/start", tag = "auction",
    params(("id" = i64, Path)),
    responses(
        (status = 200, body = AuctionSigning),
        (status = 401, body = ErrorBody), (status = 403, body = ErrorBody), (status = 404, body = ErrorBody),
        (status = 409, body = ErrorBody)
    )
)]
pub async fn start_auction(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
) -> ApiResult<Json<AuctionSigning>> {
    let (user_id, a) = own_draft(&state, &session, id).await?;
    recent_passkey(&session).await?;
    let settings = enabled(&state).await?;
    check_can_auction(&state, user_id, Some(a.forum_id)).await?;
    let w = &state.wallet;
    let domain = w.domain()?;
    // 钱包或合约变了（重新部署）就不能再用这份草稿。
    if domain.verifying_contract.0.as_slice() != a.verifying_contract.as_slice()
        || w.chain_id() != a.chain_id
    {
        return Err(ApiError(ErrorCode::AuctionUnavailable));
    }
    let pricing = pricing_of(&a)?;
    let terms = terms_of(&a)?;
    let now = OffsetDateTime::now_utc();
    let duration = u32::try_from(a.duration_secs).map_err(ApiError::internal)?;
    let auth = AuctionAuth {
        seller: address(&a.seller)?,
        token: w.usdt()?,
        auction_id: U256::from_u128(u128::try_from(a.id).map_err(ApiError::internal)?),
        reserve_price: U256::from_u128(num(&a.reserve_price)?),
        currency: pricing.currency.code(),
        cny_per_usdt: pricing.cny_per_usdt,
        fee_payer: terms.fee_payer,
        // 按当前收费：卖家接受的上限就是现在的费率、固定 gas 费与最低手续费。
        max_fee_bps: w.limits.fee_bps,
        max_base_gas: U256::from_u128(w.limits.base_gas.0),
        max_min_fee: U256::from_u128(w.limits.min_fee.0),
        instant: terms.instant,
        ship_window: terms.ship_window,
        confirm_window: terms.confirm_window,
        listing_hash: B256(
            a.content_hash
                .as_slice()
                .try_into()
                .map_err(ApiError::internal)?,
        ),
        expiry: settings.auth_expiry(
            u64::try_from(now.unix_timestamp()).map_err(ApiError::internal)?,
            duration,
        ),
    };
    let auth_hash = auth.hash(&domain);
    let sign_deadline = now + time::Duration::seconds(i64::from(SIGN_WINDOW_SECS));
    let secs = |v: u32| i32::try_from(v).map_err(ApiError::internal);
    let ok = db::start_signing(
        &state.db,
        a.id,
        user_id,
        db::SignStart {
            auth: &serde_json::to_value(auth).map_err(ApiError::internal)?,
            auth_hash: &auth_hash.0,
            sign_deadline,
            snipe_window_secs: secs(settings.snipe_window_secs)?,
            snipe_extension_secs: secs(settings.snipe_extension_secs)?,
            extension_cap_secs: secs(settings.extension_cap_secs)?,
        },
    )
    .await?;
    if !ok {
        return Err(ApiError(ErrorCode::AuctionUnavailable));
    }
    let (name, fields) = typed_fields(AUCTION_TYPE);
    Ok(Json(AuctionSigning {
        auction_id: a.id,
        typed_data: AuctionTypedData {
            domain: Eip712Domain {
                name: DOMAIN_NAME,
                version: DOMAIN_VERSION,
                chain_id: domain.chain_id,
                verifying_contract: Some(domain.verifying_contract.to_string()),
            },
            types: BTreeMap::from([(name, fields)]),
            primary_type: "AuctionAuth",
            message: auth,
        },
        auth_hash: auth_hash.to_string(),
        sign_deadline,
    }))
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct FinishAuctionRequest {
    /// 卖家对 `auth_hash` 的签名（`0x` 十六进制；智能账户为 ERC-1271 / ERC-6492 格式）。
    pub signature: String,
}

/// 挂拍第三步：验签（与合约相同的规则，经链上读节点），通过后发拍卖帖、开始计时。
#[utoipa::path(
    post, path = "/api/v1/auctions/{id}/finish", tag = "auction",
    params(("id" = i64, Path)),
    request_body = FinishAuctionRequest,
    responses(
        (status = 200, body = AuctionView),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody), (status = 409, body = ErrorBody), (status = 503, body = ErrorBody)
    )
)]
pub async fn finish_auction(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
    Json(req): Json<FinishAuctionRequest>,
) -> ApiResult<Json<AuctionView>> {
    let (user_id, a) = own_draft(&state, &session, id).await?;
    enabled(&state).await?;
    // 发帖与挂拍的处罚在建草稿之后也可能落下：上线（发帖）时再查一次。
    check_sanctions(&state, user_id, Some(a.forum_id)).await?;
    check_can_auction(&state, user_id, Some(a.forum_id)).await?;
    let (Some(auth_hash), Some(deadline), Some(draft)) =
        (a.auth_hash.as_deref(), a.sign_deadline, a.draft.as_ref())
    else {
        return Err(ApiError(ErrorCode::AuctionUnavailable));
    };
    if deadline <= OffsetDateTime::now_utc() {
        return Err(ApiError(ErrorCode::AuctionSignExpired));
    }
    let signature = parse_signature(&req.signature)?;
    let hash = B256(auth_hash.try_into().map_err(ApiError::internal)?);
    let valid = state
        .wallet
        .auction_chain
        .signature_valid(&address(&a.seller)?, &hash, &signature)
        .await
        .map_err(|e| unavailable(&e))?;
    if !valid {
        return Err(ApiError(ErrorCode::AuctionSignatureInvalid));
    }
    let draft: Draft = serde_json::from_value(draft.clone()).map_err(ApiError::internal)?;
    let forum = forum_db::forums::by_id(&state.db, a.forum_id)
        .await?
        .ok_or(ApiError(ErrorCode::NotFound))?;
    // 建草稿之后站长可能关掉了这个板块的拍卖
    if !forum.allow_auction {
        return Err(ApiError(ErrorCode::AuctionNotAllowed));
    }
    let mut tx = state.db.begin().await?;
    let created = topics::create(
        &mut *tx,
        topics::NewTopic {
            forum_id: a.forum_id,
            author_id: user_id,
            title: &draft.title,
            html: &draft.html,
            plain_text: &draft.plain_text,
            source: &draft.source,
            mentions: &draft.mentions,
            kind: topics::KIND_AUCTION,
        },
    )
    .await?;
    if !db::go_live(&mut *tx, a.id, auth_hash, &signature, created.topic_id).await? {
        // 期间重新开始了签名（授权换了）或超时：主题随事务一起回滚。
        return Err(ApiError(ErrorCode::AuctionSignExpired));
    }
    tx.commit().await?;
    crate::posting::after_commit(
        "board_touch",
        state.cache.board_touch(
            a.forum_id,
            created.topic_id,
            crate::posting::micros(created.last_post_at),
        ),
    )
    .await;
    guest::listing_changed(&state, created.topic_id, &[a.forum_id]).await;
    let a = db::by_id(&state.db, id)
        .await?
        .ok_or_else(|| ApiError::internal("auction vanished"))?;
    Ok(Json(AuctionView::of(&state, &a, Some(user_id)).await?))
}

/// 卖家取消：草稿随时可以；进行中只能在第一口出价之前。
#[utoipa::path(
    post, path = "/api/v1/auctions/{id}/cancel", tag = "auction",
    params(("id" = i64, Path)),
    responses(
        (status = 200, body = AuctionView),
        (status = 401, body = ErrorBody), (status = 403, body = ErrorBody), (status = 404, body = ErrorBody),
        (status = 409, body = ErrorBody)
    )
)]
pub async fn cancel_auction(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
) -> ApiResult<Json<AuctionView>> {
    let user_id = current_user_id(&session).await?;
    enabled(&state).await?;
    let (a, viewer) = visible(&state, &session, id).await?;
    if a.seller_id != user_id {
        return Err(ApiError(ErrorCode::Forbidden));
    }
    if !db::cancel(&state.db, id, user_id).await? {
        let code = if a.bid_count > 0 {
            ErrorCode::AuctionHasBids
        } else {
            ErrorCode::AuctionUnavailable
        };
        return Err(ApiError(code));
    }
    let a = db::by_id(&state.db, id)
        .await?
        .ok_or_else(|| ApiError::internal("auction vanished"))?;
    if let Some(topic_id) = a.topic_id {
        publish_state(&state, topic_id, &a).await;
    }
    view(&state, &a, &viewer).await
}

/// 拍卖状态变了：推给打开着主题页的连接，并让游客缓存里带这条拍卖信息行的列表失效。
async fn publish_state(state: &AppState, topic_id: i64, a: &Auction) {
    guest::listing_changed(state, topic_id, &[a.forum_id]).await;
    state
        .hub
        .publish(
            Route::Watchers {
                topic: Topic::Thread(topic_id),
            },
            &Event::AuctionState {
                topic_id,
                auction_id: a.id,
                state: state_of(a, OffsetDateTime::now_utc()).as_str().to_owned(),
            },
        )
        .await;
}

// ───────────────────────────── 出价 ─────────────────────────────

/// 公开出价记录里的出价人：用户名只露首尾各一个字。
pub fn mask(username: Option<&str>) -> String {
    let chars: Vec<char> = username.unwrap_or_default().chars().collect();
    match chars.as_slice() {
        [] => "***".to_owned(),
        [one] => format!("{one}***"),
        [first, .., last] => format!("{first}***{last}"),
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BidView {
    pub id: i64,
    /// 打码的出价人。
    pub bidder: String,
    /// 是我出的价。
    pub mine: bool,
    /// 出价人与本场其他出价人是关联账号（同设备）。
    pub linked: bool,
    /// 拍卖币种最小单位，十进制字符串。
    pub amount: String,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BidPage {
    pub bids: Vec<BidView>,
    /// 还有更早的出价时为下一页的 `before`。
    pub next_cursor: Option<i64>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct BidsQuery {
    /// 上一页最后一条的 ID。
    pub before: Option<i64>,
}

/// 出价记录（新的在前，每页 50 条），公开可查，出价人打码。
#[utoipa::path(
    get, path = "/api/v1/auctions/{id}/bids", tag = "auction",
    params(("id" = i64, Path), BidsQuery),
    responses((status = 200, body = BidPage), (status = 404, body = ErrorBody))
)]
pub async fn list_bids(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
    Query(q): Query<BidsQuery>,
) -> ApiResult<Json<BidPage>> {
    let (a, viewer) = visible(&state, &session, id).await?;
    let me = viewer.user.map(|u| u.id);
    let rows = db::bids(&state.db, a.id, q.before, BIDS_PAGE).await?;
    let next_cursor = (rows.len() == BIDS_PAGE as usize)
        .then(|| rows.last().map(|r| r.id))
        .flatten();
    Ok(Json(BidPage {
        bids: rows
            .into_iter()
            .map(|r| BidView {
                id: r.id,
                bidder: mask(r.username.as_deref()),
                mine: Some(r.bidder_id) == me,
                linked: r.linked,
                amount: r.amount,
                created_at: r.created_at,
            })
            .collect(),
        next_cursor,
    }))
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BidRequest {
    /// 拍卖币种最小单位（USDT 最小单位或分），十进制字符串。
    pub amount: String,
}

/// 钱包 USDT 余额（链上 latest，缓存几秒）。
async fn balance(state: &AppState, wallet: &Address) -> ApiResult<Usdt> {
    let key = format!("{}auction-balance:{wallet}", state.cache.ns());
    if let Some(hit) = ephemeral::get(&state.kv, &key).await? {
        return Ok(Usdt(num(&hit)?));
    }
    let b = state
        .wallet
        .auction_chain
        .usdt_balance(wallet)
        .await
        .map_err(|e| unavailable(&e))?;
    ephemeral::put(&state.kv, &key, &b.0.to_string(), BALANCE_TTL).await?;
    Ok(b)
}

/// 出价。点出价就生效、不花 gas（plan 第 7 节「拍卖模式」）。
#[utoipa::path(
    post, path = "/api/v1/auctions/{id}/bids", tag = "auction",
    params(("id" = i64, Path)),
    request_body = BidRequest,
    responses(
        (status = 201, body = AuctionView),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody), (status = 409, body = ErrorBody), (status = 429, body = ErrorBody),
        (status = 503, body = ErrorBody)
    )
)]
pub async fn place_bid(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
    Json(req): Json<BidRequest>,
) -> ApiResult<(StatusCode, Json<AuctionView>)> {
    let user_id = current_user_id(&session).await?;
    let settings = enabled(&state).await?;
    limit(&state, &BID_RATE, user_id).await?;
    let (a, viewer) = visible(&state, &session, id).await?;
    let actor = viewer.require()?;
    let now = OffsetDateTime::now_utc();
    if state_of(&a, now) != AuctionState::Live {
        return Err(ApiError(ErrorCode::AuctionUnavailable));
    }
    if a.seller_id == user_id || db::linked(&state.db, a.seller_id, user_id).await? {
        return Err(ApiError(ErrorCode::BidBySeller));
    }
    check_can_auction(&state, user_id, Some(a.forum_id)).await?;
    let wallet = wallets::wallet_on(&state.db, user_id, a.chain_id)
        .await?
        .ok_or(ApiError(ErrorCode::WalletRequired))?;
    let amount = usdt_of(&req.amount)?.0;
    let pricing = pricing_of(&a)?;
    let amount_usdt = pricing.to_usdt(amount)?;
    if amount_usdt > state.wallet.limits.order_cap {
        return Err(ApiError(ErrorCode::AmountOutOfRange));
    }
    let new_account =
        now - actor.created_at < time::Duration::days(i64::from(settings.new_account_days));
    if new_account && amount_usdt > settings.new_account_cap() {
        return Err(ApiError(ErrorCode::BidNewAccountCap));
    }
    // 先按读到的当前价给出明确的错误；最终以出价语句里的原子检查为准。
    if a.top_bidder_id == Some(user_id) {
        return Err(ApiError(ErrorCode::BidAlreadyLeading));
    }
    let top = a.top_amount.as_deref().map(num).transpose()?;
    if amount < min_next_bid(num(&a.start_price)?, num(&a.increment)?, top, &pricing)? {
        return Err(ApiError(ErrorCode::BidTooLow));
    }
    if let Some(required) = eligibility_of(&a)?.required(amount_usdt)
        && balance(&state, &address(&wallet.address)?).await? < required
    {
        return Err(ApiError(ErrorCode::BidBalanceInsufficient));
    }

    let (amount_s, usdt_s) = (amount.to_string(), amount_usdt.0.to_string());
    let one_usdt = pricing.one_usdt().to_string();
    let mut tx = state.db.begin().await?;
    let Some(placed) = db::place_bid(
        &mut *tx,
        db::NewBid {
            auction_id: a.id,
            bidder_id: user_id,
            amount: &amount_s,
            amount_usdt: &usdt_s,
            one_usdt: &one_usdt,
        },
    )
    .await?
    else {
        drop(tx);
        // 被并发的出价抢先，或刚好结束：重读给出原因。
        let now = db::by_id(&state.db, id)
            .await?
            .ok_or(ApiError(ErrorCode::NotFound))?;
        let code = if state_of(&now, OffsetDateTime::now_utc()) != AuctionState::Live {
            ErrorCode::AuctionUnavailable
        } else if now.top_bidder_id == Some(user_id) {
            ErrorCode::BidAlreadyLeading
        } else {
            ErrorCode::BidTooLow
        };
        return Err(ApiError(code));
    };
    if let Some(prev) = placed.prev_bidder.filter(|&p| p != user_id) {
        db::notify(
            &mut *tx,
            prev,
            &a,
            db::EVENT_OUTBID,
            json!({ "amount": amount_s, "currency": pricing.currency }),
        )
        .await?;
    }
    tx.commit().await?;
    if let Some(topic_id) = a.topic_id {
        // 列表里的拍卖信息行（当前价、出价次数、结束时间）变了
        guest::listing_changed(&state, topic_id, &[a.forum_id]).await;
        let username = forum_db::users::find_by_id(&state.db, user_id)
            .await?
            .and_then(|u| u.username);
        state
            .hub
            .publish(
                Route::Watchers {
                    topic: Topic::Thread(topic_id),
                },
                &Event::AuctionBid {
                    topic_id,
                    auction_id: a.id,
                    bid_id: placed.bid_id,
                    amount: amount_s,
                    bidder: mask(username.as_deref()),
                    linked: placed.linked,
                    bids: placed.bid_count,
                    ends_at: placed.ends_at,
                    reserve_met: placed.reserve_met,
                },
            )
            .await;
    }
    let a = db::by_id(&state.db, id)
        .await?
        .ok_or_else(|| ApiError::internal("auction vanished"))?;
    Ok((
        StatusCode::CREATED,
        Json(AuctionView::of(&state, &a, Some(user_id)).await?),
    ))
}

// ───────────────────────────── 付款 ─────────────────────────────

#[derive(Debug, Serialize, ToSchema)]
pub struct AuctionPayment {
    pub auction_id: i64,
    /// 第几份成交证明（1 = 赢家，2、3 = 递补）。
    pub round: i16,
    /// `createFromAuction(auth, sellerSig, proof, platformSig)` 的合约地址与链。
    pub escrow: String,
    pub chain_id: i64,
    pub auth: AuctionAuth,
    pub seller_signature: String,
    pub proof: SettlementProof,
    pub platform_signature: String,
    pub currency: Currency,
    pub cny_per_usdt: Option<i32>,
    /// 成交价（拍卖币种最小单位）。
    pub price: String,
    /// 订单金额（USDT 最小单位）：人民币按卖家汇率向上取整，与合约换算一致。
    pub amount: String,
    /// 按当前收费估算：需要 `approve` 并转入的总额 = 金额 + 买家手续费 + 买家 gas 费（以链上建单时为准）。
    pub buyer_pays: String,
    pub buyer_fee: String,
    pub buyer_gas: String,
    /// 付款必须从这个地址发起（成交证明里的赢家）。
    pub buyer_wallet: String,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub pay_deadline: OffsetDateTime,
}

/// 中标者付款需要的数据：卖家授权与签名、平台成交证明与签名、应付 USDT。只有当前这份证明的中标者能取，
/// 且在付款截止时间之前。
#[utoipa::path(
    get, path = "/api/v1/auctions/{id}/payment", tag = "auction",
    params(("id" = i64, Path)),
    responses(
        (status = 200, body = AuctionPayment),
        (status = 401, body = ErrorBody), (status = 403, body = ErrorBody), (status = 404, body = ErrorBody),
        (status = 409, body = ErrorBody)
    )
)]
pub async fn auction_payment(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
) -> ApiResult<Json<AuctionPayment>> {
    let user_id = current_user_id(&session).await?;
    let (a, _) = visible(&state, &session, id).await?;
    if a.state != db::STATE_AWAITING_PAYMENT {
        return Err(ApiError(ErrorCode::AuctionUnavailable));
    }
    let proof = db::proof(&state.db, a.id, a.round)
        .await?
        .ok_or(ApiError(ErrorCode::AuctionUnavailable))?;
    if proof.bidder_id != user_id {
        return Err(ApiError(ErrorCode::Forbidden));
    }
    if proof.pay_deadline <= OffsetDateTime::now_utc() {
        return Err(ApiError(ErrorCode::AuctionUnavailable));
    }
    let (Some(auth), Some(seller_sig)) = (a.auth.as_ref(), a.seller_signature.as_deref()) else {
        return Err(ApiError::internal("live auction without signed auth"));
    };
    let auth: AuctionAuth = serde_json::from_value(auth.clone()).map_err(ApiError::internal)?;
    let pricing = pricing_of(&a)?;
    let price = num(&proof.price)?;
    let amount = pricing.to_usdt(price)?;
    let quote = buyer_quote(&state.wallet.limits, amount, auth.fee_payer)?;
    Ok(Json(AuctionPayment {
        auction_id: a.id,
        round: proof.round,
        escrow: hex0x(&a.verifying_contract),
        chain_id: a.chain_id,
        auth,
        seller_signature: hex0x(seller_sig),
        proof: SettlementProof {
            auth_hash: B256(
                proof
                    .auth_hash
                    .as_slice()
                    .try_into()
                    .map_err(ApiError::internal)?,
            ),
            winner: address(&proof.winner)?,
            price: U256::from_u128(price),
            pay_deadline: u64::try_from(proof.pay_deadline.unix_timestamp())
                .map_err(ApiError::internal)?,
        },
        platform_signature: hex0x(&proof.signature),
        currency: pricing.currency,
        cny_per_usdt: (a.cny_per_usdt != 0).then_some(a.cny_per_usdt),
        price: proof.price.clone(),
        amount: amount.0.to_string(),
        buyer_pays: quote.buyer_pays.0.to_string(),
        buyer_fee: quote.buyer_fee.0.to_string(),
        buyer_gas: quote.buyer_gas.0.to_string(),
        buyer_wallet: hex0x(&proof.winner),
        pay_deadline: proof.pay_deadline,
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn bidder_names_are_masked() {
        assert_eq!(mask(None), "***");
        assert_eq!(mask(Some("")), "***");
        assert_eq!(mask(Some("a")), "a***");
        assert_eq!(mask(Some("ab")), "a***b");
        assert_eq!(mask(Some("小明同学")), "小***学");
    }

    #[test]
    fn typed_data_fields_follow_the_contract_typehash() {
        let (name, fields) = typed_fields(AUCTION_TYPE);
        assert_eq!(name, "AuctionAuth");
        assert_eq!(fields.len(), 15);
        assert_eq!(fields[0].name, "seller");
        assert_eq!(fields[0].kind, "address");
        assert_eq!(fields[14].name, "expiry");
        assert_eq!(fields[14].kind, "uint64");
        // 字段名与 JSON 消息的键一致（serde 驼峰）。
        let msg = serde_json::to_value(AuctionAuth {
            seller: Address::ZERO,
            token: Address::ZERO,
            auction_id: U256::ZERO,
            reserve_price: U256::ZERO,
            currency: 0,
            cny_per_usdt: 0,
            fee_payer: 0,
            max_fee_bps: 0,
            max_base_gas: U256::ZERO,
            max_min_fee: U256::ZERO,
            instant: false,
            ship_window: 0,
            confirm_window: 0,
            listing_hash: B256::ZERO,
            expiry: 0,
        })
        .unwrap();
        let keys: Vec<&String> = msg.as_object().unwrap().keys().collect();
        for f in &fields {
            assert!(keys.contains(&&f.name), "{}", f.name);
        }
    }
}
