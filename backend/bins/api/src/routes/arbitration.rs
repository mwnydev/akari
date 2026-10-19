//! 仲裁（plan 第 7 节「仲裁规则」、ADR 0013 第 2、4、5 条）：案件详情与证据、当事人陈述、裁决（论坛裁决 Passkey 确认）、
//! 申诉、站长终裁 / 叫停、仲裁 Safe 执行数据、公示页数据。状态机与分配规则见 `forum_db::arbitration`。
//!
//! 谁能看到什么：
//! - 当事人（买卖双方）：订单、双方陈述、自己能做的操作；裁决公示后才看到裁决与理由，任何时候都看不到仲裁员是谁。
//!   当事人即使同时是站长或管理员，也只按当事人对待。
//! - 被分配的仲裁员（被改派走的不算）与站长：全部证据、聊天与哈希链校验结果、分配与裁决记录。
//!   两人裁决的案件在两人都提交之前，仲裁员只看到自己的分配与裁决（看不到另一人是谁、交没交、判了什么）。
//! - 其他人：一律 404（不暴露案件是否存在）；公示页只给已公示的裁决，且不含任何身份信息。
//!
//! 裁决的 Passkey 确认：`ruling/options` 按提交的裁决与链上订单算出 [`RulingIntent`]，把它的哈希设为 WebAuthn challenge，
//! 状态存 Valkey 5 分钟；`rulings` 用存下的随机数按提交内容重算，不一致即拒绝（`RULING_MISMATCH`），一致再校验断言。
//! 聊天哈希链校验不通过时拒绝裁决并提醒站长（T-16）。

use std::time::Duration;

use alloy::primitives::Address;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use forum_auth::passkey::bind_challenge;
use forum_auth::passkey::webauthn::{
    PasskeyAuthentication, PublicKeyCredential, RequestChallengeResponse,
};
use forum_chain::arbitration::SafeCall;
use forum_chain::order::Market;
use forum_core::ErrorCode;
use forum_core::arbitration::{
    MAX_STATEMENT_UPLOADS, MAX_TEXT_CHARS, RulingIntent, RulingKind, allocate,
};
use forum_core::chat::OrderMessage;
use forum_db::arbitration::{
    self as db, Assignment, Case, NewRuling, Outcome, ROLE_APPEAL, ROLE_PANEL, Ruling,
};
use forum_db::orders::STATE_DISPUTED;
use forum_kv::SlidingWindow;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;
use tower_sessions::Session;
use utoipa::{IntoParams, ToSchema};

use super::me::current_user_id;
use super::order_chat::ChainVerification;
use super::orders::OrderView;
use super::ruling_passkey::{CEREMONY_RATE, parse_passkey, require_eligible, verify_assertion};
use super::uploads::UploadView;
use super::wallet::{save_challenge, take_challenge};
use crate::arbitration::RulingOrigin;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::posting::limit;
use crate::state::AppState;
use crate::views::{UserCardView, hex0x};

const STATEMENT_RATE: SlidingWindow =
    SlidingWindow::new("arbitration-statement", 20, Duration::from_secs(3600));
/// 每位当事人每个案件最多提交的陈述条数（陈述只追加，限制总量也限制了详情页的读取量）。
const MAX_STATEMENTS_PER_PARTY: usize = 20;
/// 聊天记录读入详情的批大小。
const CHAT_BATCH: i64 = 1000;

/// 裁决签名的 WebAuthn 仪式按案件区分：同一人同时打开两个案件的签名不会互相覆盖。
fn ruling_challenge(case_id: i64) -> String {
    format!("arbitration:ruling:{case_id}")
}

fn not_found() -> ApiError {
    ApiError(ErrorCode::NotFound)
}

/// 去掉首尾空白后 1–2000 字。
fn text(s: &str) -> ApiResult<&str> {
    let t = s.trim();
    if t.is_empty() {
        return Err(ApiError(ErrorCode::ContentEmpty));
    }
    if t.chars().count() > MAX_TEXT_CHARS {
        return Err(ApiError(ErrorCode::ContentTooLong));
    }
    Ok(t)
}

/// 访问者在案件里的身份。
struct Seat {
    me: i64,
    /// `buyer` / `seller`；当事人只按当事人对待。
    party: Option<&'static str>,
    owner: bool,
    /// 本人在本案的分配（被改派走的不算）。
    assignment: Option<Assignment>,
}

impl Seat {
    fn staff(&self) -> bool {
        self.party.is_none() && (self.owner || self.assignment.is_some())
    }

    fn active(&self, role: &str) -> Option<&Assignment> {
        self.assignment
            .as_ref()
            .filter(|a| a.is_active() && a.role == role)
    }

    /// 两人裁决、两人都提交之前的仲裁员：只能看到自己的。
    fn blind(&self, case: &Case) -> bool {
        case.status == db::STATUS_REVIEWING
            && case.panel_size == 2
            && self
                .assignment
                .as_ref()
                .is_some_and(|a| a.role == ROLE_PANEL)
    }
}

async fn seat(
    state: &AppState,
    case: &Case,
    assignments: &[Assignment],
    me: i64,
) -> ApiResult<Seat> {
    let party = if case.buyer_id == Some(me) {
        Some("buyer")
    } else if case.seller_id == Some(me) {
        Some("seller")
    } else {
        None
    };
    let seat = Seat {
        me,
        party,
        owner: party.is_none() && db::is_owner(&state.db, me).await?,
        assignment: assignments
            .iter()
            .find(|a| a.arbitrator_id == me && a.counts())
            .cloned(),
    };
    if seat.party.is_none() && !seat.staff() {
        return Err(not_found());
    }
    Ok(seat)
}

/// 裁决前的利益冲突检查：与当事人有关联或 90 天内有交易的回避（接案后才产生的关联也算）；申诉复核还要求没参与过。
struct Conflicts {
    recused: bool,
    took_part: bool,
}

async fn conflicts(state: &AppState, case: &Case, seat: &Seat) -> ApiResult<Conflicts> {
    if !seat.staff() {
        return Ok(Conflicts {
            recused: true,
            took_part: true,
        });
    }
    Ok(Conflicts {
        recused: db::is_recused(&state.db, case.id, seat.me).await?,
        took_part: db::took_part(&state.db, case.id, seat.me).await?,
    })
}

/// 这份裁决能不能由这个人、在案件当前状态下提交；能则返回对应的分配。
fn check_kind(
    case: &Case,
    seat: &Seat,
    c: &Conflicts,
    kind: RulingKind,
    now: OffsetDateTime,
) -> ApiResult<Option<i64>> {
    if !seat.staff() || c.recused {
        return Err(ApiError(ErrorCode::Forbidden));
    }
    if !case.is_open() || case.order_state != STATE_DISPUTED {
        return Err(ApiError(ErrorCode::CaseStateInvalid));
    }
    let status = case.status.as_str();
    match kind {
        RulingKind::Panel => {
            let a = seat
                .active(ROLE_PANEL)
                .ok_or(ApiError(ErrorCode::Forbidden))?;
            if status != db::STATUS_REVIEWING {
                return Err(ApiError(ErrorCode::CaseStateInvalid));
            }
            if now < case.evidence_until {
                return Err(ApiError(ErrorCode::EvidenceWindowOpen));
            }
            Ok(Some(a.id))
        }
        RulingKind::Owner => {
            if !seat.owner {
                return Err(ApiError(ErrorCode::Forbidden));
            }
            // 申诉中叫停的案件不能由站长直接改判（复核必须由没参与的人做），只能恢复后走复核。
            let halted_ok = status == db::STATUS_HALTED
                && case.halted_from.as_deref() != Some(db::STATUS_APPEALED);
            if status != db::STATUS_OWNER_REVIEW && !halted_ok {
                return Err(ApiError(ErrorCode::CaseStateInvalid));
            }
            Ok(None)
        }
        RulingKind::Appeal => {
            if let Some(a) = seat.active(ROLE_APPEAL) {
                if status != db::STATUS_APPEALED {
                    return Err(ApiError(ErrorCode::CaseStateInvalid));
                }
                return Ok(Some(a.id));
            }
            if !seat.owner || c.took_part {
                return Err(ApiError(ErrorCode::Forbidden));
            }
            if status != db::STATUS_APPEALED {
                return Err(ApiError(ErrorCode::CaseStateInvalid));
            }
            Ok(None)
        }
    }
}

async fn load(state: &AppState, id: i64, me: i64) -> ApiResult<(Case, Vec<Assignment>, Seat)> {
    let case = db::case_by_id(&state.db, id).await?.ok_or_else(not_found)?;
    let assignments = db::assignments(&state.db, id).await?;
    let seat = seat(state, &case, &assignments, me).await?;
    Ok((case, assignments, seat))
}

// ───────────────────────────── 视图 ─────────────────────────────

#[derive(Debug, Serialize, ToSchema)]
pub struct CaseStatement {
    pub id: i64,
    /// 提交的当事人（买家或卖家的用户 ID）。
    pub user_id: i64,
    pub body: String,
    /// 附带的图片（只含处理完成的）。
    pub uploads: Vec<UploadView>,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CaseRuling {
    pub id: i64,
    pub kind: RulingKind,
    /// 作出裁决的人；只给仲裁员与站长看。
    pub arbitrator_id: Option<i64>,
    pub to_buyer_bps: i32,
    pub bond_to_buyer_bps: i32,
    /// 争议本金判给买家 / 卖家的金额（最小单位，未计手续费、gas 与争议费的结算）。
    pub principal_to_buyer: String,
    pub principal_to_seller: String,
    pub bond_to_buyer: String,
    /// 公开理由。
    pub reason: String,
    /// 是否当前生效（已公示）的裁决。
    pub effective: bool,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

impl CaseRuling {
    fn of(r: &Ruling, case: &Case, with_identity: bool) -> Self {
        Self {
            id: r.id,
            kind: RulingKind::parse(&r.kind).unwrap_or(RulingKind::Panel),
            arbitrator_id: with_identity.then_some(r.arbitrator_id),
            to_buyer_bps: r.to_buyer_bps,
            bond_to_buyer_bps: r.bond_to_buyer_bps,
            principal_to_buyer: r.principal_to_buyer.clone(),
            principal_to_seller: r.principal_to_seller.clone(),
            bond_to_buyer: r.bond_to_buyer.clone(),
            reason: r.reason.clone(),
            effective: case.final_ruling_id == Some(r.id),
            created_at: r.created_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CaseAssignment {
    pub arbitrator_id: i64,
    /// `panel`（裁决）或 `appeal`（申诉复核）。
    pub role: String,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub assigned_at: OffsetDateTime,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub due_at: OffsetDateTime,
    /// `ruled`（已提交）或 `reassigned`（超时改派）；在岗为空。
    pub ended_reason: Option<String>,
}

/// 订单聊天全文与哈希链校验结果（仲裁员与站长）。
#[derive(Debug, Serialize, ToSchema)]
pub struct CaseChat {
    pub messages: Vec<OrderMessage>,
    pub verification: ChainVerification,
}

/// 访问者能做的操作。
#[derive(Debug, Serialize, ToSchema)]
pub struct CaseActions {
    /// `buyer` / `seller` / `panel` / `appeal` / `owner`。
    pub seat: String,
    /// 能提交的裁决（举证期未结束时也给出，提交时返回 `EVIDENCE_WINDOW_OPEN`）。
    pub rule: Option<RulingKind>,
    pub statement: bool,
    pub appeal: bool,
    pub halt: bool,
    pub resume: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CaseHalt {
    pub by: i64,
    pub reason: Option<String>,
}

/// 仲裁 Safe 要执行的交易：签名人在 Safe 官方界面导入 `transaction_builder` 后各自签名（3 人 2/3）。
#[derive(Debug, Serialize, ToSchema)]
pub struct SafeExecution {
    pub case_id: i64,
    pub chain_id: i64,
    /// 仲裁 Safe 地址（未配置时为空，签名人在界面里自己选 Safe）。
    pub safe: Option<String>,
    /// 目标担保合约。
    pub to: String,
    pub value: String,
    /// `resolve(orderId, toBuyerBps, bondToBuyerBps)` 的 calldata。
    pub data: String,
    /// 0 = CALL。
    pub operation: u8,
    pub chain_order_id: i64,
    pub to_buyer_bps: i32,
    pub bond_to_buyer_bps: i32,
    /// Safe「Transaction Builder」可直接导入的 JSON 文件。
    #[schema(value_type = Object)]
    pub transaction_builder: Value,
}

/// 争议超过 `resolveTimeout` 未裁决时任何人可调用的兜底结算（担保合约里争议中的订单不能再延长期限，
/// `extendDeadlines` 只对未进入争议的订单有效，所以这里没有延期操作）。
#[derive(Debug, Serialize, ToSchema)]
pub struct TimeoutAction {
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    /// 现在是否已可调用。
    pub callable: bool,
    pub to: String,
    /// `resolveTimeout(orderId)` 的 calldata。
    pub data: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CaseDetail {
    pub id: i64,
    /// `reviewing` / `owner_review` / `appeal_window` / `appealed` / `ready` / `halted`。
    pub status: String,
    /// 裁决人数（≥ 200 USDT 两人）。
    pub panel_size: i16,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub evidence_until: OffsetDateTime,
    #[schema(value_type = Option<String>, format = DateTime)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub published_at: Option<OffsetDateTime>,
    #[schema(value_type = Option<String>, format = DateTime)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub appeal_until: Option<OffsetDateTime>,
    /// 申诉人（买家或卖家的用户 ID）与理由。
    pub appeal_by: Option<i64>,
    pub appeal_reason: Option<String>,
    /// 叫停人与理由（仲裁员与站长可见）。
    pub halt: Option<CaseHalt>,
    #[schema(value_type = Option<String>, format = DateTime)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub closed_at: Option<OffsetDateTime>,
    /// `executed`（Safe 已执行）/ `settled`（和解或超时兜底）/ `reverted`（争议被区块回滚撤销）。
    pub close_reason: Option<String>,
    pub order: OrderView,
    pub buyer: Option<UserCardView>,
    pub seller: Option<UserCardView>,
    pub statements: Vec<CaseStatement>,
    /// 按可见性过滤后的裁决（见模块说明）。
    pub rulings: Vec<CaseRuling>,
    /// 仲裁员与站长可见（盲审期间只有自己的）。
    pub assignments: Vec<CaseAssignment>,
    /// 仲裁员与站长可见。
    pub chat: Option<CaseChat>,
    /// 可执行（`ready`）时给仲裁员与站长。
    pub execution: Option<SafeExecution>,
    pub timeout: Option<TimeoutAction>,
    pub actions: CaseActions,
}

fn market(case: &Case) -> ApiResult<(Market, Address, u64)> {
    let market = Market::from_mode(case.mode).map_err(ApiError::internal)?;
    let contract = Address::try_from(case.contract.as_slice()).map_err(ApiError::internal)?;
    let order_id = u64::try_from(case.chain_order_id).map_err(ApiError::internal)?;
    Ok((market, contract, order_id))
}

fn resolve_call(case: &Case, ruling: &Ruling) -> ApiResult<SafeCall> {
    let (market, contract, order_id) = market(case)?;
    let bps = |b: i32| u32::try_from(b).map_err(ApiError::internal);
    Ok(forum_chain::arbitration::resolve_call(
        market,
        contract,
        order_id,
        bps(ruling.to_buyer_bps)?,
        bps(ruling.bond_to_buyer_bps)?,
    ))
}

/// 一组裁决（同一条链）的 Safe 交易文件。
fn builder(
    state: &AppState,
    chain_id: i64,
    name: &str,
    items: &[(&Case, &Ruling)],
    calls: &[SafeCall],
) -> ApiResult<Value> {
    Ok(forum_chain::arbitration::transaction_builder(
        u64::try_from(chain_id).map_err(ApiError::internal)?,
        state.arbitration.safe,
        name,
        &batch_description(items),
        calls,
        now_ms(),
    ))
}

fn execution(state: &AppState, case: &Case, ruling: &Ruling) -> ApiResult<SafeExecution> {
    let call = resolve_call(case, ruling)?;
    let transaction_builder = builder(
        state,
        case.chain_id,
        &format!("仲裁案件 #{}", case.id),
        &[(case, ruling)],
        std::slice::from_ref(&call),
    )?;
    Ok(SafeExecution {
        case_id: case.id,
        chain_id: case.chain_id,
        safe: state.arbitration.safe.map(|a| a.to_checksum(None)),
        to: call.to.to_checksum(None),
        value: "0".to_owned(),
        data: call.data.to_string(),
        operation: 0,
        chain_order_id: case.chain_order_id,
        to_buyer_bps: ruling.to_buyer_bps,
        bond_to_buyer_bps: ruling.bond_to_buyer_bps,
        transaction_builder,
    })
}

fn batch_description(items: &[(&Case, &Ruling)]) -> String {
    items
        .iter()
        .map(|(c, r)| {
            format!(
                "案件 #{} 订单 {} resolve({}, {}, {})",
                c.id, c.chain_order_id, c.chain_order_id, r.to_buyer_bps, r.bond_to_buyer_bps
            )
        })
        .collect::<Vec<_>>()
        .join("；")
}

fn now_ms() -> i64 {
    i64::try_from(OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000).unwrap_or(0)
}

fn timeout_action(case: &Case, now: OffsetDateTime) -> ApiResult<Option<TimeoutAction>> {
    let Some(at) = case.timeout_at() else {
        return Ok(None);
    };
    if !case.is_open() {
        return Ok(None);
    }
    let (market, contract, order_id) = market(case)?;
    let call = forum_chain::arbitration::resolve_timeout_call(market, contract, order_id);
    Ok(Some(TimeoutAction {
        at,
        callable: now > at,
        to: call.to.to_checksum(None),
        data: call.data.to_string(),
    }))
}

async fn statements(state: &AppState, case_id: i64) -> ApiResult<Vec<CaseStatement>> {
    let mut out = Vec::new();
    for s in db::statements(&state.db, case_id).await? {
        let mut uploads = Vec::new();
        for id in &s.upload_ids {
            if let Some(u) = forum_db::uploads::by_id(&state.db, *id).await?
                && u.user_id == s.user_id
            {
                uploads.push(UploadView::try_from(u)?);
            }
        }
        out.push(CaseStatement {
            id: s.id,
            user_id: s.user_id,
            body: s.body,
            uploads,
            created_at: s.created_at,
        });
    }
    Ok(out)
}

async fn chat(state: &AppState, order_id: i64) -> ApiResult<CaseChat> {
    let mut messages = Vec::new();
    loop {
        let after = messages.last().map_or(0, |m: &OrderMessage| m.id);
        let batch = forum_db::order_chat::after(&state.db, order_id, after, CHAT_BATCH).await?;
        let full = i64::try_from(batch.len()) == Ok(CHAT_BATCH);
        messages.extend(batch);
        if !full {
            break;
        }
    }
    let verification = ChainVerification::of(&forum_core::chat::verify(&messages));
    Ok(CaseChat {
        messages,
        verification,
    })
}

async fn detail(
    state: &AppState,
    case: Case,
    assignments: Vec<Assignment>,
    seat: Seat,
) -> ApiResult<CaseDetail> {
    let now = OffsetDateTime::now_utc();
    let staff = seat.staff();
    let blind = seat.blind(&case);
    let conflicts = conflicts(state, &case, &seat).await?;
    let all = db::rulings(&state.db, case.id).await?;
    let rulings: Vec<CaseRuling> = if staff {
        all.iter()
            .filter(|r| !blind || r.arbitrator_id == seat.me)
            .map(|r| CaseRuling::of(r, &case, true))
            .collect()
    } else if case.published_at.is_some() {
        all.iter()
            .map(|r| CaseRuling::of(r, &case, false))
            .collect()
    } else {
        Vec::new()
    };
    let assignments = if staff {
        assignments
            .iter()
            .filter(|a| !blind || a.arbitrator_id == seat.me)
            .map(|a| CaseAssignment {
                arbitrator_id: a.arbitrator_id,
                role: a.role.clone(),
                assigned_at: a.assigned_at,
                due_at: a.due_at,
                ended_reason: a.ended_reason.clone(),
            })
            .collect()
    } else {
        Vec::new()
    };
    let execution = match case.final_ruling_id {
        Some(id) if staff && case.status == db::STATUS_READY && case.is_open() => all
            .iter()
            .find(|r| r.id == id)
            .map(|r| execution(state, &case, r))
            .transpose()?,
        _ => None,
    };
    let rule = [RulingKind::Panel, RulingKind::Owner, RulingKind::Appeal]
        .into_iter()
        .find(|k| {
            matches!(
                check_kind(&case, &seat, &conflicts, *k, now),
                Ok(_) | Err(ApiError(ErrorCode::EvidenceWindowOpen))
            )
        });
    let party_open = seat.party.is_some() && case.is_open();
    let can_halt = seat.owner
        && case.is_open()
        && [
            db::STATUS_APPEAL_WINDOW,
            db::STATUS_APPEALED,
            db::STATUS_READY,
        ]
        .contains(&case.status.as_str());
    let actions = CaseActions {
        seat: match (seat.party, &seat.assignment) {
            (Some(p), _) => p.to_owned(),
            (None, Some(a)) => a.role.clone(),
            (None, None) => "owner".to_owned(),
        },
        rule,
        statement: party_open && now < case.evidence_until,
        appeal: party_open
            && case.status == db::STATUS_APPEAL_WINDOW
            && case.appeal_by.is_none()
            && case.appeal_until.is_some_and(|t| now < t),
        halt: can_halt,
        resume: seat.owner && case.is_open() && case.status == db::STATUS_HALTED,
    };
    let order = forum_db::orders::by_ids(&state.db, &[case.order_id])
        .await?
        .pop()
        .ok_or_else(|| ApiError::internal("case without order"))?;
    let mut cards = crate::cards::load(
        state,
        &[case.buyer_id, case.seller_id]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>(),
    )
    .await?;
    let mut card = |id: Option<i64>| {
        id.and_then(|id| cards.iter().position(|c| c.id == id))
            .map(|i| cards.swap_remove(i))
    };
    let (buyer, seller) = (card(case.buyer_id), card(case.seller_id));
    Ok(CaseDetail {
        id: case.id,
        status: case.status.clone(),
        panel_size: case.panel_size,
        evidence_until: case.evidence_until,
        published_at: case.published_at,
        appeal_until: case.appeal_until,
        appeal_by: case.appeal_by,
        appeal_reason: case.appeal_reason.clone(),
        halt: case
            .halted_by
            .filter(|_| staff && case.status == db::STATUS_HALTED)
            .map(|by| CaseHalt {
                by,
                reason: case.halt_reason.clone(),
            }),
        closed_at: case.closed_at,
        close_reason: case.close_reason.clone(),
        order: OrderView::of(order)?,
        buyer,
        seller,
        statements: statements(state, case.id).await?,
        rulings,
        assignments,
        chat: if staff {
            Some(chat(state, case.order_id).await?)
        } else {
            None
        },
        execution,
        timeout: timeout_action(&case, now)?,
        actions,
    })
}

// ───────────────────────────── 接口 ─────────────────────────────

/// 案件详情与证据（当事人、本案仲裁员、站长；见模块说明的可见性规则）。
#[utoipa::path(
    get, path = "/api/v1/arbitration/cases/{id}", tag = "arbitration",
    params(("id" = i64, Path)),
    responses((status = 200, body = CaseDetail), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody))
)]
pub async fn get_case(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
) -> ApiResult<Json<CaseDetail>> {
    let me = current_user_id(&session).await?;
    let (case, assignments, seat) = load(&state, id, me).await?;
    Ok(Json(detail(&state, case, assignments, seat).await?))
}

/// 订单的仲裁案件（订单页跳转用）；没有案件或无权查看都返回 404。
#[utoipa::path(
    get, path = "/api/v1/orders/{id}/case", tag = "arbitration", operation_id = "order_case",
    params(("id" = i64, Path)),
    responses((status = 200, body = CaseDetail), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody))
)]
pub async fn order_case(
    State(state): State<AppState>,
    session: Session,
    Path(order_id): Path<i64>,
) -> ApiResult<Json<CaseDetail>> {
    let me = current_user_id(&session).await?;
    let id = db::case_id_by_order(&state.db, order_id)
        .await?
        .ok_or_else(not_found)?;
    let (case, assignments, seat) = load(&state, id, me).await?;
    Ok(Json(detail(&state, case, assignments, seat).await?))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct QueueEntry {
    pub case_id: i64,
    pub order_id: i64,
    pub status: String,
    /// `panel` / `appeal`。
    pub role: String,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub due_at: OffsetDateTime,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub evidence_until: OffsetDateTime,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OwnerQueueEntry {
    pub case_id: i64,
    pub status: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ArbitrationQueue {
    /// 我在岗的分配，按期限排序。
    pub assignments: Vec<QueueEntry>,
    /// 站长：待终裁、待复核、已叫停、待执行与找不到人接案的（非站长为空）。
    pub owner: Vec<OwnerQueueEntry>,
}

/// 仲裁待办（站长与管理员）。
#[utoipa::path(
    get, path = "/api/v1/arbitration/queue", tag = "arbitration",
    responses((status = 200, body = ArbitrationQueue), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody))
)]
pub async fn arbitration_queue(
    State(state): State<AppState>,
    session: Session,
) -> ApiResult<Json<ArbitrationQueue>> {
    let me = current_user_id(&session).await?;
    require_eligible(&state, me).await?;
    let assignments = db::queue(&state.db, me)
        .await?
        .into_iter()
        .map(|q| QueueEntry {
            case_id: q.case_id,
            order_id: q.order_id,
            status: q.status,
            role: q.role,
            due_at: q.due_at,
            evidence_until: q.evidence_until,
        })
        .collect();
    let owner = if db::is_owner(&state.db, me).await? {
        db::owner_queue(&state.db)
            .await?
            .into_iter()
            .map(|(case_id, status)| OwnerQueueEntry { case_id, status })
            .collect()
    } else {
        Vec::new()
    };
    Ok(Json(ArbitrationQueue { assignments, owner }))
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct WorkloadQuery {
    /// 统计最近多少天分配的案件（1–366，默认 30）。
    pub days: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WorkloadEntry {
    pub arbitrator_id: i64,
    pub assigned: i64,
    pub ruled: i64,
    pub on_time: i64,
    pub reassigned: i64,
    pub open: i64,
    pub owner_rulings: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Workload {
    pub days: i64,
    /// 计酬只看工作量与按时率，不含任何裁决方向（ADR 0013 第 2 条）。
    pub arbitrators: Vec<WorkloadEntry>,
}

/// 各仲裁员的工作量与按时率（站长）。
#[utoipa::path(
    get, path = "/api/v1/arbitration/workload", tag = "arbitration",
    params(WorkloadQuery),
    responses(
        (status = 200, body = Workload), (status = 400, body = ErrorBody), (status = 401, body = ErrorBody),
        (status = 403, body = ErrorBody)
    )
)]
pub async fn arbitration_workload(
    State(state): State<AppState>,
    session: Session,
    Query(q): Query<WorkloadQuery>,
) -> ApiResult<Json<Workload>> {
    let me = current_user_id(&session).await?;
    if !db::is_owner(&state.db, me).await? {
        return Err(ApiError(ErrorCode::Forbidden));
    }
    let days = q.days.unwrap_or(30);
    if !(1..=366).contains(&days) {
        return Err(ApiError(ErrorCode::InvalidParameter));
    }
    let since = OffsetDateTime::now_utc() - time::Duration::days(days);
    let arbitrators = db::workload(&state.db, since)
        .await?
        .into_iter()
        .map(|w| WorkloadEntry {
            arbitrator_id: w.arbitrator_id,
            assigned: w.assigned,
            ruled: w.ruled,
            on_time: w.on_time,
            reassigned: w.reassigned,
            open: w.open,
            owner_rulings: w.owner_rulings,
        })
        .collect();
    Ok(Json(Workload { days, arbitrators }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewStatement {
    /// 1–2000 字。
    pub body: String,
    /// 自己上传且已处理完成的图片（`POST /api/v1/uploads`），最多 9 张。
    #[serde(default)]
    pub upload_ids: Vec<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct StatementCreated {
    pub id: i64,
}

/// 当事人在举证期内提交陈述与证据（只追加，双方与仲裁员都能看到）。
#[utoipa::path(
    post, path = "/api/v1/arbitration/cases/{id}/statements", tag = "arbitration",
    params(("id" = i64, Path)),
    request_body = NewStatement,
    responses(
        (status = 201, body = StatementCreated), (status = 400, body = ErrorBody), (status = 401, body = ErrorBody),
        (status = 403, body = ErrorBody), (status = 404, body = ErrorBody), (status = 409, body = ErrorBody),
        (status = 429, body = ErrorBody)
    )
)]
pub async fn add_case_statement(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
    Json(req): Json<NewStatement>,
) -> ApiResult<(StatusCode, Json<StatementCreated>)> {
    let me = current_user_id(&session).await?;
    let (case, _, seat) = load(&state, id, me).await?;
    if seat.party.is_none() {
        return Err(ApiError(ErrorCode::Forbidden));
    }
    let body = text(&req.body)?;
    let mut uploads = req.upload_ids.clone();
    uploads.sort_unstable();
    uploads.dedup();
    if uploads.len() > MAX_STATEMENT_UPLOADS
        || db::own_done_uploads(&state.db, me, &uploads).await?
            != i64::try_from(uploads.len()).unwrap_or(-1)
    {
        return Err(ApiError(ErrorCode::InvalidParameter));
    }
    limit(&state, &STATEMENT_RATE, me).await?;
    let mut tx = state.db.begin().await?;
    db::lock_case(&mut tx, id).await?;
    let case = db::case_by_id(&mut *tx, id).await?.unwrap_or(case);
    if !case.is_open() {
        return Err(ApiError(ErrorCode::CaseStateInvalid));
    }
    if OffsetDateTime::now_utc() >= case.evidence_until {
        return Err(ApiError(ErrorCode::EvidenceWindowClosed));
    }
    let mine = db::statements(&mut *tx, id)
        .await?
        .iter()
        .filter(|s| s.user_id == me)
        .count();
    if mine >= MAX_STATEMENTS_PER_PARTY {
        return Err(ApiError(ErrorCode::RateLimited));
    }
    let sid = db::add_statement(&mut tx, id, me, body, &uploads).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(StatementCreated { id: sid })))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RulingRequest {
    /// `panel`（被分配的仲裁员）/ `owner`（站长终裁、叫停后改判）/ `appeal`（申诉复核）。
    pub kind: RulingKind,
    /// 争议本金判给买家的比例（0–10000），其余归卖家。
    pub to_buyer_bps: u32,
    /// 锁定的卖家保证金赔给买家的比例（0–10000）；订单没有保证金时必须为 0。
    pub bond_to_buyer_bps: u32,
    /// 公开理由，1–2000 字。
    pub reason: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RulingOptions {
    /// 原样传给 `navigator.credentials.get()`；challenge = `sha256("akari-ruling\0" ‖ intent)`。
    #[schema(value_type = Object)]
    pub options: RequestChallengeResponse,
    /// 这次确认签署的内容（界面照此展示「你将判给买家 …」）。
    pub intent: String,
    /// `0x` + 64 位十六进制。
    pub challenge: String,
}

#[derive(Serialize, Deserialize)]
struct PendingRuling {
    auth: PasskeyAuthentication,
    nonce: [u8; 32],
    challenge: [u8; 32],
}

/// 校验并算出这份裁决的签署内容（`nonce` 为这次确认的随机数）。
async fn intent(
    state: &AppState,
    case: &Case,
    seat: &Seat,
    req: &RulingRequest,
    nonce: &[u8; 32],
) -> ApiResult<(Option<i64>, RulingIntent, String)> {
    let conflicts = conflicts(state, case, seat).await?;
    let assignment = check_kind(case, seat, &conflicts, req.kind, OffsetDateTime::now_utc())?;
    let reason = text(&req.reason)?.to_owned();
    let allocation = allocate(case.disputed()?, req.to_buyer_bps, req.bond_to_buyer_bps)?;
    let intent = RulingIntent::new(
        case.id,
        req.kind,
        case.chain_id,
        &case.contract,
        case.chain_order_id,
        req.to_buyer_bps,
        req.bond_to_buyer_bps,
        allocation,
        &reason,
        nonce,
    );
    Ok((assignment, intent, reason))
}

/// 开始用裁决 Passkey 确认一份裁决：challenge 绑定这份裁决的全部内容。
#[utoipa::path(
    post, path = "/api/v1/arbitration/cases/{id}/ruling/options", tag = "arbitration",
    params(("id" = i64, Path)),
    request_body = RulingRequest,
    responses(
        (status = 200, body = RulingOptions), (status = 400, body = ErrorBody), (status = 401, body = ErrorBody),
        (status = 403, body = ErrorBody), (status = 404, body = ErrorBody), (status = 409, body = ErrorBody),
        (status = 429, body = ErrorBody)
    )
)]
pub async fn ruling_options(
    State(state): State<AppState>,
    _: RulingOrigin,
    session: Session,
    Path(id): Path<i64>,
    Json(req): Json<RulingRequest>,
) -> ApiResult<Json<RulingOptions>> {
    let me = current_user_id(&session).await?;
    let (case, _, seat) = load(&state, id, me).await?;
    let nonce: [u8; 32] = rand::rng().random();
    let (_, intent, _) = intent(&state, &case, &seat, &req, &nonce).await?;
    if forum_db::order_chat::verify_chain(&state.db, case.order_id)
        .await?
        .broken_at
        .is_some()
    {
        return Err(ApiError(ErrorCode::ChatChainBroken));
    }
    let stored = db::ruling_passkeys(&state.db, me).await?;
    if stored.is_empty() {
        return Err(ApiError(ErrorCode::PasskeyRequired));
    }
    limit(&state, &CEREMONY_RATE, me).await?;
    let creds = stored
        .iter()
        .map(parse_passkey)
        .collect::<ApiResult<Vec<_>>>()?;
    let (mut options, auth) = state
        .arbitration
        .webauthn
        .start_passkey_authentication(&creds)
        .map_err(ApiError::internal)?;
    let challenge = intent.challenge();
    let auth = bind_challenge(&mut options, &auth, &challenge).map_err(ApiError::internal)?;
    save_challenge(
        &state,
        &ruling_challenge(id),
        me,
        &PendingRuling {
            auth,
            nonce,
            challenge,
        },
    )
    .await?;
    Ok(Json(RulingOptions {
        options,
        intent: intent.canonical(),
        challenge: hex0x(&challenge),
    }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RulingSubmit {
    pub kind: RulingKind,
    pub to_buyer_bps: u32,
    pub bond_to_buyer_bps: u32,
    pub reason: String,
    /// `navigator.credentials.get()` 的结果（用 `ruling/options` 给的选项）。
    #[schema(value_type = Object)]
    pub credential: PublicKeyCredential,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RulingSubmitted {
    pub ruling_id: i64,
    /// `waiting`（等另一人）/ `disagreed`（转站长终裁）/ `published`（已公示，进入申诉期）/ `final`（可执行）。
    pub outcome: String,
    pub status: String,
}

/// 提交裁决：内容必须与 `ruling/options` 确认的完全一致，断言必须由本人的裁决 Passkey 签出，聊天哈希链必须完好。
#[utoipa::path(
    post, path = "/api/v1/arbitration/cases/{id}/rulings", tag = "arbitration",
    params(("id" = i64, Path)),
    request_body = RulingSubmit,
    responses(
        (status = 201, body = RulingSubmitted), (status = 400, body = ErrorBody), (status = 401, body = ErrorBody),
        (status = 403, body = ErrorBody), (status = 404, body = ErrorBody), (status = 409, body = ErrorBody)
    )
)]
pub async fn submit_ruling(
    State(state): State<AppState>,
    _: RulingOrigin,
    session: Session,
    Path(id): Path<i64>,
    Json(req): Json<RulingSubmit>,
) -> ApiResult<(StatusCode, Json<RulingSubmitted>)> {
    let me = current_user_id(&session).await?;
    // 先取出（作废）确认状态：无论成败，同一次确认只能用一次。
    let pending: PendingRuling = take_challenge(&state, &ruling_challenge(id), me).await?;
    let (case, _, seat) = load(&state, id, me).await?;
    let ruling = RulingRequest {
        kind: req.kind,
        to_buyer_bps: req.to_buyer_bps,
        bond_to_buyer_bps: req.bond_to_buyer_bps,
        reason: req.reason.clone(),
    };
    let (_, intent, reason) = intent(&state, &case, &seat, &ruling, &pending.nonce).await?;
    let challenge = intent.challenge();
    if challenge != pending.challenge {
        return Err(ApiError(ErrorCode::RulingMismatch));
    }
    let passkey_id = verify_assertion(&state, me, &req.credential, &pending.auth).await?;
    let check = forum_db::order_chat::verify_chain(&state.db, case.order_id).await?;
    if let Some(at) = check.broken_at {
        let mut tx = state.db.begin().await?;
        db::audit(
            &mut *tx,
            me,
            "arbitration.chat_chain_broken",
            id,
            None,
            &json!({ "broken_at": at, "count": check.count }),
        )
        .await?;
        db::alert_owners(
            &mut tx,
            "chat_chain_broken",
            id,
            json!({ "status": case.status }),
        )
        .await?;
        tx.commit().await?;
        return Err(ApiError(ErrorCode::ChatChainBroken));
    }
    let assertion = serde_json::to_value(&req.credential).map_err(ApiError::internal)?;
    let mut tx = state.db.begin().await?;
    db::lock_case(&mut tx, id).await?;
    // 锁住后按最新状态再检查一次（例如另一位仲裁员刚提交、站长刚叫停）。
    let case = db::case_by_id(&mut *tx, id).await?.ok_or_else(not_found)?;
    let assignments = db::assignments(&mut *tx, id).await?;
    let seat = Seat {
        assignment: assignments
            .iter()
            .find(|a| a.arbitrator_id == me && a.counts())
            .cloned(),
        ..seat
    };
    let conflicts = conflicts(&state, &case, &seat).await?;
    let assignment_id = check_kind(
        &case,
        &seat,
        &conflicts,
        req.kind,
        OffsetDateTime::now_utc(),
    )?;
    let signed = intent.canonical();
    let bps = |b: u32| i32::try_from(b).map_err(|_| ApiError(ErrorCode::InvalidParameter));
    let (ruling_id, outcome) = db::submit_ruling(
        &mut tx,
        &case,
        &NewRuling {
            case_id: id,
            arbitrator_id: me,
            assignment_id,
            kind: req.kind,
            to_buyer_bps: bps(req.to_buyer_bps)?,
            bond_to_buyer_bps: bps(req.bond_to_buyer_bps)?,
            principal_to_buyer: intent.principal_to_buyer.clone(),
            principal_to_seller: intent.principal_to_seller.clone(),
            bond_to_buyer: intent.bond_to_buyer.clone(),
            reason: &reason,
            chat_head: &check.head(),
            chat_count: i64::try_from(check.count).unwrap_or(i64::MAX),
            signed_intent: &signed,
            challenge: &challenge,
            passkey_id,
            assertion: &assertion,
        },
    )
    .await?;
    tx.commit().await?;
    let status = db::case_by_id(&state.db, id)
        .await?
        .map(|c| c.status)
        .unwrap_or_default();
    Ok((
        StatusCode::CREATED,
        Json(RulingSubmitted {
            ruling_id,
            outcome: match outcome {
                Outcome::Waiting => "waiting",
                Outcome::Disagreed => "disagreed",
                Outcome::Published => "published",
                Outcome::Final => "final",
            }
            .to_owned(),
            status,
        }),
    ))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ReasonRequest {
    /// 1–2000 字。
    pub reason: String,
}

/// 当事人申诉：裁决公示后 72 小时内、每个案件一次，由没参与过的仲裁员（或站长）复核，复核结果为最终裁决。
#[utoipa::path(
    post, path = "/api/v1/arbitration/cases/{id}/appeal", tag = "arbitration",
    params(("id" = i64, Path)),
    request_body = ReasonRequest,
    responses(
        (status = 204), (status = 400, body = ErrorBody), (status = 401, body = ErrorBody),
        (status = 403, body = ErrorBody), (status = 404, body = ErrorBody), (status = 409, body = ErrorBody)
    )
)]
pub async fn appeal_case(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
    Json(req): Json<ReasonRequest>,
) -> ApiResult<StatusCode> {
    let me = current_user_id(&session).await?;
    let (_, _, seat) = load(&state, id, me).await?;
    if seat.party.is_none() {
        return Err(ApiError(ErrorCode::Forbidden));
    }
    let reason = text(&req.reason)?;
    let mut tx = state.db.begin().await?;
    db::lock_case(&mut tx, id).await?;
    let case = db::case_by_id(&mut *tx, id).await?.ok_or_else(not_found)?;
    let open = case.is_open()
        && case.status == db::STATUS_APPEAL_WINDOW
        && case.appeal_by.is_none()
        && case
            .appeal_until
            .is_some_and(|t| OffsetDateTime::now_utc() < t);
    if !open {
        return Err(ApiError(ErrorCode::CaseStateInvalid));
    }
    db::appeal(&mut tx, &case, me, reason).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// 站长在执行前叫停（公示后、Safe 执行前）。之后可恢复，或以站长终裁改判（重新公示）。
#[utoipa::path(
    post, path = "/api/v1/arbitration/cases/{id}/halt", tag = "arbitration",
    params(("id" = i64, Path)),
    request_body = ReasonRequest,
    responses(
        (status = 204), (status = 400, body = ErrorBody), (status = 401, body = ErrorBody),
        (status = 403, body = ErrorBody), (status = 404, body = ErrorBody), (status = 409, body = ErrorBody)
    )
)]
pub async fn halt_case(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
    Json(req): Json<ReasonRequest>,
) -> ApiResult<StatusCode> {
    let me = current_user_id(&session).await?;
    let (_, _, seat) = load(&state, id, me).await?;
    if !seat.owner {
        return Err(ApiError(ErrorCode::Forbidden));
    }
    let reason = text(&req.reason)?;
    let mut tx = state.db.begin().await?;
    db::lock_case(&mut tx, id).await?;
    let case = db::case_by_id(&mut *tx, id).await?.ok_or_else(not_found)?;
    let haltable = [
        db::STATUS_APPEAL_WINDOW,
        db::STATUS_APPEALED,
        db::STATUS_READY,
    ]
    .contains(&case.status.as_str());
    if !case.is_open() || !haltable {
        return Err(ApiError(ErrorCode::CaseStateInvalid));
    }
    db::halt(&mut tx, &case, me, reason).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// 站长解除叫停，回到叫停前的状态。
#[utoipa::path(
    post, path = "/api/v1/arbitration/cases/{id}/resume", tag = "arbitration",
    params(("id" = i64, Path)),
    responses(
        (status = 204), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody), (status = 409, body = ErrorBody)
    )
)]
pub async fn resume_case(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let me = current_user_id(&session).await?;
    let (_, _, seat) = load(&state, id, me).await?;
    if !seat.owner {
        return Err(ApiError(ErrorCode::Forbidden));
    }
    let mut tx = state.db.begin().await?;
    db::lock_case(&mut tx, id).await?;
    let case = db::case_by_id(&mut *tx, id).await?.ok_or_else(not_found)?;
    if !case.is_open() || case.status != db::STATUS_HALTED {
        return Err(ApiError(ErrorCode::CaseStateInvalid));
    }
    db::resume(&mut tx, &case, me).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// 一条链上待执行的全部裁决，合成一个 Safe 批量交易文件（每天合并一笔执行）。
#[derive(Debug, Serialize, ToSchema)]
pub struct ExecutionBatch {
    pub chain_id: i64,
    pub safe: Option<String>,
    pub cases: Vec<SafeExecution>,
    #[schema(value_type = Object)]
    pub transaction_builder: Value,
}

/// 全部可执行裁决的 Safe 交易数据（站长与管理员，即仲裁 Safe 的签名人）。
#[utoipa::path(
    get, path = "/api/v1/arbitration/execution", tag = "arbitration",
    responses((status = 200, body = [ExecutionBatch]), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody))
)]
pub async fn execution_batch(
    State(state): State<AppState>,
    session: Session,
) -> ApiResult<Json<Vec<ExecutionBatch>>> {
    let me = current_user_id(&session).await?;
    require_eligible(&state, me).await?;
    let mut ready = Vec::new();
    for id in db::ready_case_ids(&state.db).await? {
        let Some(case) = db::case_by_id(&state.db, id).await? else {
            continue;
        };
        let Some(ruling) = db::rulings(&state.db, id)
            .await?
            .into_iter()
            .find(|r| Some(r.id) == case.final_ruling_id)
        else {
            continue;
        };
        ready.push((case, ruling));
    }
    let mut chains: Vec<i64> = ready.iter().map(|(c, _)| c.chain_id).collect();
    chains.sort_unstable();
    chains.dedup();
    let mut out = Vec::new();
    for chain_id in chains {
        let items: Vec<(&Case, &Ruling)> = ready
            .iter()
            .filter(|(c, _)| c.chain_id == chain_id)
            .map(|(c, r)| (c, r))
            .collect();
        let calls = items
            .iter()
            .map(|(c, r)| resolve_call(c, r))
            .collect::<ApiResult<Vec<_>>>()?;
        out.push(ExecutionBatch {
            chain_id,
            safe: state.arbitration.safe.map(|a| a.to_checksum(None)),
            transaction_builder: builder(
                &state,
                chain_id,
                &format!("仲裁裁决批量执行（{} 件）", items.len()),
                &items,
                &calls,
            )?,
            cases: items
                .iter()
                .map(|(c, r)| execution(&state, c, r))
                .collect::<ApiResult<_>>()?,
        });
    }
    Ok(Json(out))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PublicRuling {
    pub case_id: i64,
    pub status: String,
    /// `escrow` / `stream`。
    pub mode: &'static str,
    /// 订单金额（最小单位）与代币精度。
    pub amount: String,
    pub token_decimals: i16,
    #[schema(value_type = Option<String>, format = DateTime)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub published_at: Option<OffsetDateTime>,
    #[schema(value_type = Option<String>, format = DateTime)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub appeal_until: Option<OffsetDateTime>,
    pub appealed: bool,
    pub close_reason: Option<String>,
    /// 全部裁决与理由（不含任何人的身份；`effective` 为当前生效的那份）。
    pub rulings: Vec<CaseRuling>,
}

/// 裁决公示页数据（无需登录；只有已公示的案件，不含当事人与仲裁员的身份、地址）。
#[utoipa::path(
    get, path = "/api/v1/arbitration/public/{id}", tag = "arbitration",
    params(("id" = i64, Path)),
    responses((status = 200, body = PublicRuling), (status = 404, body = ErrorBody))
)]
pub async fn public_ruling(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<PublicRuling>> {
    let case = db::case_by_id(&state.db, id)
        .await?
        .filter(|c| c.published_at.is_some())
        .ok_or_else(not_found)?;
    let rulings = db::rulings(&state.db, id)
        .await?
        .iter()
        .map(|r| CaseRuling::of(r, &case, false))
        .collect();
    let (market, _, _) = market(&case)?;
    Ok(Json(PublicRuling {
        case_id: case.id,
        status: case.status.clone(),
        mode: market.as_str(),
        amount: case.amount.clone(),
        token_decimals: case.token_decimals,
        published_at: case.published_at,
        appeal_until: case.appeal_until,
        appealed: case.appeal_by.is_some(),
        close_reason: case.close_reason.clone(),
        rulings,
    }))
}
