//! 处罚（ADR 0013）：禁言、禁拍（版主只在自己板块内，分别最长 7 天、30 天）、封号、IP / 设备封禁（管理员），
//! 查看与解除。只能处罚角色比自己低的人；每次操作限速、计入异常检测并写审计日志。
//!
//! 封号的效果：不能发帖、回帖、评论、点赞、举报、编辑、发私信；仍可登录、查看订单与钱包、提币（托管资金永远能取回）。
//! IP / 设备封禁只拦登录（发验证码与校验），配合封号阻止换号。

use std::net::IpAddr;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use forum_core::ErrorCode;
use forum_core::staff::{Permission, Scope};
use forum_db::sanctions::{self, NewSanction, Sanction};
use forum_db::{Postgres, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};
use tower_sessions::Session;
use utoipa::{IntoParams, ToSchema};

use crate::access::Viewer;
use crate::cards;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::moderation::{self, Class, Moderator, action, denied};
use crate::state::AppState;

/// 处罚时长下限 1 分钟、上限 10 年（更长用永久）。
const MIN_SECS: i64 = 60;
const MAX_SECS: i64 = 3650 * 86_400;
/// IP 封禁的最大网段：IPv4 /16、IPv6 /32，防止误封一大片（包括管理人员自己）。
const MIN_PREFIX_V4: u8 = 16;
const MIN_PREFIX_V6: u8 = 32;
/// 用户处罚记录最多返回条数。
const LIST_LIMIT: i64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SanctionKind {
    Mute,
    AuctionBan,
    Ban,
    IpBan,
    DeviceBan,
}

impl SanctionKind {
    fn id(self) -> i16 {
        match self {
            Self::Mute => sanctions::KIND_MUTE,
            Self::AuctionBan => sanctions::KIND_AUCTION_BAN,
            Self::Ban => sanctions::KIND_BAN,
            Self::IpBan => sanctions::KIND_IP_BAN,
            Self::DeviceBan => sanctions::KIND_DEVICE_BAN,
        }
    }

    fn from_id(id: i16) -> Option<Self> {
        [
            Self::Mute,
            Self::AuctionBan,
            Self::Ban,
            Self::IpBan,
            Self::DeviceBan,
        ]
        .into_iter()
        .find(|k| k.id() == id)
    }

    fn permission(self) -> Permission {
        match self {
            Self::Mute => Permission::Mute,
            Self::AuctionBan => Permission::AuctionBan,
            Self::Ban => Permission::Ban,
            Self::IpBan => Permission::IpBan,
            Self::DeviceBan => Permission::DeviceBan,
        }
    }

    fn action(self) -> &'static str {
        match self {
            Self::Mute => action::MUTE,
            Self::AuctionBan => action::AUCTION_BAN,
            Self::Ban => action::BAN,
            Self::IpBan => action::IP_BAN,
            Self::DeviceBan => action::DEVICE_BAN,
        }
    }

    /// 范围：只有禁言、禁拍可以限定板块。
    fn scope(self, forum_id: Option<i32>) -> ApiResult<Scope> {
        match (self, forum_id) {
            (Self::Mute | Self::AuctionBan, Some(f)) => Ok(Scope::Board(f)),
            (_, None) => Ok(Scope::Site),
            (_, Some(_)) => Err(ApiError(ErrorCode::InvalidParameter)),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SanctionView {
    pub id: i64,
    pub kind: SanctionKind,
    pub user_id: Option<i64>,
    /// 只限这个板块；为空表示全站。
    pub forum_id: Option<i32>,
    /// IP 封禁的网段。
    pub ip: Option<String>,
    /// 为空表示永久。
    #[schema(value_type = Option<String>, format = DateTime)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub ends_at: Option<OffsetDateTime>,
    pub reason: String,
    /// 处罚人；系统自动的处罚（拍卖违约禁拍）为空。
    pub created_by: Option<i64>,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[schema(value_type = Option<String>, format = DateTime)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub revoked_at: Option<OffsetDateTime>,
    /// 仍在生效（未解除、未到期）。
    pub active: bool,
}

impl SanctionView {
    fn of(s: Sanction, now: OffsetDateTime) -> Option<Self> {
        Some(Self {
            active: s.active(now),
            id: s.id,
            kind: SanctionKind::from_id(s.kind)?,
            user_id: s.user_id,
            forum_id: s.forum_id,
            ip: s.ip,
            ends_at: s.ends_at,
            reason: s.reason,
            created_by: s.created_by,
            created_at: s.created_at,
            revoked_at: s.revoked_at,
        })
    }
}

/// 处罚用户的条款（处罚接口与举报处理共用）。
#[derive(Debug, Clone, Copy, Deserialize, ToSchema)]
pub struct SanctionTerms {
    /// `mute` / `auction_ban` / `ban` / `device_ban`（IP 封禁用 `POST /api/v1/moderation/ip-bans`）。
    pub kind: SanctionKind,
    /// 禁言、禁拍只限这个板块；为空表示全站（版主必须填自己负责的板块）。
    pub forum_id: Option<i32>,
    /// 时长（秒，60 秒到 10 年）；为空表示永久。版主禁言最长 7 天、禁拍最长 30 天，且不能永久。
    pub duration_secs: Option<i64>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SanctionRequest {
    #[serde(flatten)]
    pub terms: SanctionTerms,
    /// 预设理由或补充说明，写入审计日志并通知对方。
    pub reason: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SanctionsCreated {
    /// 新处罚的 ID（设备封禁按设备各一条）。
    pub ids: Vec<i64>,
    #[schema(value_type = Option<String>, format = DateTime)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub ends_at: Option<OffsetDateTime>,
}

/// 校验时长并换算成到期时间；为空表示永久。
fn ends_at(
    m: &Moderator,
    p: Permission,
    scope: Scope,
    secs: Option<i64>,
) -> ApiResult<Option<OffsetDateTime>> {
    if secs.is_some_and(|s| !(MIN_SECS..=MAX_SECS).contains(&s)) {
        return Err(ApiError(ErrorCode::InvalidParameter));
    }
    let limit = m
        .roles
        .duration_limit(p, scope)
        .ok_or_else(|| denied(&m.roles))?;
    if !limit.allows(secs) {
        return Err(ApiError(ErrorCode::SanctionTooLong));
    }
    Ok(secs.map(|s| OffsetDateTime::now_utc() + Duration::seconds(s)))
}

fn rfc3339(t: Option<OffsetDateTime>) -> ApiResult<Option<String>> {
    t.map(|t| t.format(&Rfc3339).map_err(ApiError::internal))
        .transpose()
}

/// 只能处罚（或解除）角色比自己低的人，也不能处罚自己。
async fn check_target(state: &AppState, m: &Moderator, user_id: i64) -> ApiResult<()> {
    let target = forum_db::staff::roles(&state.db, user_id).await?;
    if user_id == m.id || !m.roles.outranks(&target) {
        return Err(ApiError(ErrorCode::Forbidden));
    }
    Ok(())
}

/// 处罚用户：禁言、禁拍、封号、封禁其登录过的全部设备。
#[utoipa::path(
    post, path = "/api/v1/moderation/users/{id}/sanctions", tag = "moderation",
    params(("id" = i64, Path)),
    request_body = SanctionRequest,
    responses(
        (status = 201, body = SanctionsCreated),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody), (status = 429, body = ErrorBody)
    )
)]
pub async fn sanction_user(
    State(state): State<AppState>,
    session: Session,
    Path(user_id): Path<i64>,
    Json(req): Json<SanctionRequest>,
) -> ApiResult<(StatusCode, Json<SanctionsCreated>)> {
    let actor = Viewer::load(&state, &session).await?.require()?;
    let reason = moderation::reason(&req.reason)?;
    let (m, ends_at) = authorize_sanction(&state, actor.id, user_id, req.terms).await?;
    let mut tx = state.db.begin().await?;
    m.guard(&state, &mut tx).await?;
    let ids = apply_sanction(&mut tx, actor.id, user_id, req.terms, ends_at, reason).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(SanctionsCreated { ids, ends_at })))
}

/// 核对一次处罚（事务之前）：权限与限速、对象与板块存在、只能处罚角色比自己低的人、时长在权限内。
/// 返回已授权的操作者与到期时间（为空表示永久）。
pub(crate) async fn authorize_sanction(
    state: &AppState,
    actor_id: i64,
    user_id: i64,
    terms: SanctionTerms,
) -> ApiResult<(Moderator, Option<OffsetDateTime>)> {
    if terms.kind == SanctionKind::IpBan {
        return Err(ApiError(ErrorCode::InvalidParameter));
    }
    let scope = terms.kind.scope(terms.forum_id)?;
    let p = terms.kind.permission();
    let m = Moderator::authorize(state, actor_id, p, scope, Class::Sanction).await?;
    forum_db::users::find_by_id(&state.db, user_id)
        .await?
        .ok_or(ApiError(ErrorCode::NotFound))?;
    if let Some(f) = terms.forum_id {
        forum_db::forums::by_id(&state.db, f)
            .await?
            .ok_or(ApiError(ErrorCode::NotFound))?;
    }
    check_target(state, &m, user_id).await?;
    let ends_at = ends_at(&m, p, scope, terms.duration_secs)?;
    Ok((m, ends_at))
}

/// 在管理事务里（已做过异常检测）写处罚、审计日志与给对方的通知，返回新处罚的 ID。处罚接口与举报处理共用。
pub(crate) async fn apply_sanction(
    tx: &mut Transaction<'_, Postgres>,
    actor_id: i64,
    user_id: i64,
    terms: SanctionTerms,
    ends_at: Option<OffsetDateTime>,
    reason: &str,
) -> ApiResult<Vec<i64>> {
    let ids = if terms.kind == SanctionKind::DeviceBan {
        let ids =
            sanctions::create_device_bans(&mut **tx, user_id, ends_at, reason, actor_id).await?;
        if ids.is_empty() {
            return Err(ApiError(ErrorCode::NotFound));
        }
        ids
    } else {
        let s = NewSanction {
            kind: terms.kind.id(),
            user_id,
            forum_id: terms.forum_id,
            ends_at,
            reason,
            created_by: Some(actor_id),
        };
        vec![sanctions::create(&mut **tx, s).await?]
    };
    let until = rfc3339(ends_at)?;
    forum_db::audit::append(
        &mut **tx,
        actor_id,
        terms.kind.action(),
        Some(&format!("user:{user_id}")),
        Some(reason),
        &json!({ "sanction_ids": ids, "forum_id": terms.forum_id, "ends_at": until }),
    )
    .await?;
    if terms.kind != SanctionKind::DeviceBan {
        forum_db::outbox::enqueue(
            &mut **tx,
            "notify.sanction",
            &json!({
                "to": user_id, "kind": terms.kind, "forum_id": terms.forum_id, "ends_at": until, "reason": reason,
            }),
            forum_db::outbox::PRIORITY_NORMAL,
        )
        .await?;
    }
    Ok(ids)
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct IpBanRequest {
    /// 单个 IP（如 `203.0.113.7`）或网段（如 `203.0.113.0/24`，IPv4 最大 /16、IPv6 最大 /32）。
    pub ip: String,
    /// 时长（秒）；为空表示永久。
    pub duration_secs: Option<i64>,
    pub reason: String,
}

/// 解析 IP 或网段，清掉主机位，返回 PostgreSQL `cidr` 能接受的规范写法。
pub fn parse_cidr(raw: &str) -> Option<String> {
    let (addr, prefix) = match raw.trim().split_once('/') {
        Some((a, p)) => (a.parse::<IpAddr>().ok()?, Some(p.parse::<u8>().ok()?)),
        None => (raw.trim().parse::<IpAddr>().ok()?, None),
    };
    match addr {
        IpAddr::V4(v4) => {
            let p = prefix.unwrap_or(32);
            if !(MIN_PREFIX_V4..=32).contains(&p) {
                return None;
            }
            let mask = u32::MAX.checked_shl(32 - u32::from(p)).unwrap_or(0);
            let net = std::net::Ipv4Addr::from(u32::from(v4) & mask);
            Some(format!("{net}/{p}"))
        }
        IpAddr::V6(v6) => {
            let p = prefix.unwrap_or(128);
            if !(MIN_PREFIX_V6..=128).contains(&p) {
                return None;
            }
            let mask = u128::MAX.checked_shl(128 - u32::from(p)).unwrap_or(0);
            let net = std::net::Ipv6Addr::from(u128::from(v6) & mask);
            Some(format!("{net}/{p}"))
        }
    }
}

/// 封禁 IP 或网段：之后从这里发起的登录（发验证码、校验验证码）一律拒绝。
#[utoipa::path(
    post, path = "/api/v1/moderation/ip-bans", tag = "moderation",
    request_body = IpBanRequest,
    responses(
        (status = 201, body = SanctionsCreated),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 429, body = ErrorBody)
    )
)]
pub async fn ban_ip(
    State(state): State<AppState>,
    session: Session,
    Json(req): Json<IpBanRequest>,
) -> ApiResult<(StatusCode, Json<SanctionsCreated>)> {
    let actor = Viewer::load(&state, &session).await?.require()?;
    let reason = moderation::reason(&req.reason)?;
    let cidr = parse_cidr(&req.ip).ok_or(ApiError(ErrorCode::InvalidParameter))?;
    let p = Permission::IpBan;
    let m = Moderator::authorize(&state, actor.id, p, Scope::Site, Class::Sanction).await?;
    let ends_at = ends_at(&m, p, Scope::Site, req.duration_secs)?;

    let mut tx = state.db.begin().await?;
    m.guard(&state, &mut tx).await?;
    let id = sanctions::create_ip_ban(&mut *tx, &cidr, ends_at, reason, actor.id).await?;
    forum_db::audit::append(
        &mut *tx,
        actor.id,
        action::IP_BAN,
        Some(&format!("ip:{cidr}")),
        Some(reason),
        &json!({ "sanction_ids": [id], "ends_at": rfc3339(ends_at)? }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(SanctionsCreated {
            ids: vec![id],
            ends_at,
        }),
    ))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SanctionList {
    /// 新的在前，最多 100 条。
    pub sanctions: Vec<SanctionView>,
}

/// 用户的处罚记录（含已解除、已到期的）。任何未被暂停的管理人员可看。
#[utoipa::path(
    get, path = "/api/v1/moderation/users/{id}/sanctions", tag = "moderation",
    params(("id" = i64, Path)),
    responses(
        (status = 200, body = SanctionList),
        (status = 401, body = ErrorBody), (status = 403, body = ErrorBody)
    )
)]
pub async fn user_sanctions(
    State(state): State<AppState>,
    session: Session,
    Path(user_id): Path<i64>,
) -> ApiResult<Json<SanctionList>> {
    let viewer = Viewer::load(&state, &session).await?;
    viewer.require()?;
    let roles = viewer.staff(&state).await?;
    if !roles.allows_anywhere(Permission::ViewSanctions) {
        return Err(denied(&roles));
    }
    let now = OffsetDateTime::now_utc();
    let rows = sanctions::of_user(&state.db, user_id, LIST_LIMIT).await?;
    Ok(Json(SanctionList {
        sanctions: rows
            .into_iter()
            .filter_map(|s| SanctionView::of(s, now))
            .collect(),
    }))
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct LiftQuery {
    /// 写入审计日志。
    pub reason: String,
}

/// 解除处罚。需要该类处罚在其范围内的权限，且角色高于被处罚人（版主解不了站长对管理员的禁言）。
#[utoipa::path(
    delete, path = "/api/v1/moderation/sanctions/{id}", tag = "moderation",
    params(("id" = i64, Path), LiftQuery),
    responses(
        (status = 204),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody), (status = 429, body = ErrorBody)
    )
)]
pub async fn lift(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
    Query(q): Query<LiftQuery>,
) -> ApiResult<StatusCode> {
    let actor = Viewer::load(&state, &session).await?.require()?;
    let reason = moderation::reason(&q.reason)?;
    let s = sanctions::by_id(&state.db, id)
        .await?
        .filter(|s| s.active(OffsetDateTime::now_utc()))
        .ok_or(ApiError(ErrorCode::NotFound))?;
    let kind = SanctionKind::from_id(s.kind).ok_or(ApiError(ErrorCode::NotFound))?;
    let scope = kind.scope(s.forum_id)?;
    let m =
        Moderator::authorize(&state, actor.id, kind.permission(), scope, Class::Sanction).await?;
    if let Some(user_id) = s.user_id {
        check_target(&state, &m, user_id).await?;
    }

    let mut tx = state.db.begin().await?;
    m.guard(&state, &mut tx).await?;
    if sanctions::revoke(&mut *tx, &[id], actor.id)
        .await?
        .is_empty()
    {
        return Err(ApiError(ErrorCode::NotFound));
    }
    forum_db::audit::append(
        &mut *tx,
        actor.id,
        action::SANCTION_LIFT,
        Some(&format!("sanction:{id}")),
        Some(reason),
        &json!({ "sanction_id": id, "kind": kind, "user_id": s.user_id }),
    )
    .await?;
    tx.commit().await?;
    // 违约带来的禁拍公开显示在名片上
    if s.kind == sanctions::KIND_AUCTION_BAN
        && let Some(user_id) = s.user_id
    {
        cards::invalidate(&state, &[user_id]).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::parse_cidr;

    #[test]
    fn cidr_normalized_and_bounded() {
        assert_eq!(parse_cidr("203.0.113.7").as_deref(), Some("203.0.113.7/32"));
        assert_eq!(
            parse_cidr(" 203.0.113.77/24 ").as_deref(),
            Some("203.0.113.0/24")
        );
        assert_eq!(parse_cidr("10.1.2.3/16").as_deref(), Some("10.1.0.0/16"));
        assert_eq!(parse_cidr("10.0.0.0/8"), None);
        assert_eq!(parse_cidr("0.0.0.0/0"), None);
        assert_eq!(
            parse_cidr("2001:db8::1/48").as_deref(),
            Some("2001:db8::/48")
        );
        assert_eq!(parse_cidr("2001:db8::/16"), None);
        assert_eq!(parse_cidr("not-an-ip"), None);
        assert_eq!(parse_cidr("1.2.3.4/33"), None);
    }
}
