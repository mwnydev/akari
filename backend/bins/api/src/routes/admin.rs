//! 站长后台的管理功能（ADR 0013）：任免管理员与版主、限速与异常阈值、恢复被暂停的管理权限、
//! 一键撤销某账号某时间段的管理操作、审计日志，以及仲裁总览与 Safe 批量执行文件。全部要求后台会话（[`AdminSession`]，只有站长），
//! 改动类操作还要求 5 分钟内用后台 Passkey 再验证；每次改动写审计日志并推送管理员群。

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use forum_core::ErrorCode;
use forum_core::staff::{ModerationSettings, Permission, Role};
use forum_db::{arbitration as arb, audit, outbox, staff};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;
use utoipa::{IntoParams, ToSchema};

use super::arbitration::{self, ExecutionBatch, Workload};
use super::users::{MentionView, search_prefix};
use crate::admin::AdminSession;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::moderation::{self, RevertEffects, RevertStatus};
use crate::state::AppState;
use crate::views::ForumView;

/// 一次撤销最多处理的操作数；更多时缩小时间段分批处理。
const REVOKE_LIMIT: i64 = 500;
/// 一次撤销的时间段最长 31 天。
const REVOKE_MAX_SPAN: time::Duration = time::Duration::days(31);
const AUDIT_PAGE: i64 = 50;

/// 敏感操作：后台权限 + 5 分钟内再验证。
fn sensitive(admin: &AdminSession, p: Permission) -> ApiResult<()> {
    admin.require(p)?;
    admin.require_recent()
}

#[derive(Debug, Serialize, ToSchema)]
pub struct StaffAssignment {
    /// 任命 ID（撤销时用）。
    pub id: i64,
    pub user_id: i64,
    pub username: Option<String>,
    pub role: Role,
    /// 版主负责的板块。
    pub forum_id: Option<i32>,
    pub granted_by: Option<i64>,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

impl StaffAssignment {
    fn of(a: staff::Assignment) -> Option<Self> {
        Some(Self {
            role: Role::from_id(a.role)?,
            id: a.id,
            user_id: a.user_id,
            username: a.username,
            forum_id: a.forum_id,
            granted_by: a.granted_by,
            created_at: a.created_at,
        })
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct StaffList {
    pub staff: Vec<StaffAssignment>,
}

/// 全部管理人员任命。
#[utoipa::path(
    get, path = "/api/v1/admin/staff", tag = "admin",
    responses((status = 200, body = StaffList), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody))
)]
pub async fn list_staff(
    State(state): State<AppState>,
    admin: AdminSession,
) -> ApiResult<Json<StaffList>> {
    admin.require(Permission::ManageStaff)?;
    let rows = staff::assignments(&state.db).await?;
    Ok(Json(StaffList {
        staff: rows.into_iter().filter_map(StaffAssignment::of).collect(),
    }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AppointRequest {
    pub user_id: i64,
    /// `admin` 或 `moderator`（站长不能经接口任命）。
    pub role: Role,
    /// 版主必填（负责的板块），管理员不填。
    pub forum_id: Option<i32>,
}

/// 任命管理员或版主。
#[utoipa::path(
    post, path = "/api/v1/admin/staff", tag = "admin",
    request_body = AppointRequest,
    responses(
        (status = 201, body = StaffAssignment),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody), (status = 409, body = ErrorBody)
    )
)]
pub async fn appoint(
    State(state): State<AppState>,
    admin: AdminSession,
    Json(req): Json<AppointRequest>,
) -> ApiResult<(StatusCode, Json<StaffAssignment>)> {
    sensitive(&admin, Permission::ManageStaff)?;
    match (req.role, req.forum_id) {
        (Role::Admin, None) => {}
        (Role::Moderator, Some(f)) => {
            forum_db::forums::by_id(&state.db, f)
                .await?
                .ok_or(ApiError(ErrorCode::NotFound))?;
        }
        _ => return Err(ApiError(ErrorCode::InvalidParameter)),
    }
    forum_db::users::find_by_id(&state.db, req.user_id)
        .await?
        .ok_or(ApiError(ErrorCode::NotFound))?;
    let mut tx = state.db.begin().await?;
    let id = staff::grant(
        &mut *tx,
        req.user_id,
        req.role.id(),
        req.forum_id,
        Some(admin.user_id),
    )
    .await?
    .ok_or(ApiError(ErrorCode::Conflict))?;
    audit::append(
        &mut *tx,
        admin.user_id,
        "staff.appoint",
        Some(&format!("user:{}", req.user_id)),
        None,
        &json!({ "assignment_id": id, "role": req.role, "forum_id": req.forum_id }),
    )
    .await?;
    let board = req
        .forum_id
        .map(|f| format!("（板块 #{f}）"))
        .unwrap_or_default();
    outbox::staff_alert(
        &mut *tx,
        "staff.appoint",
        &format!(
            "站长任命用户 #{} 为{}{board}。",
            req.user_id,
            req.role.label()
        ),
    )
    .await?;
    tx.commit().await?;
    let saved = staff::assignments(&state.db)
        .await?
        .into_iter()
        .find(|a| a.id == id)
        .and_then(StaffAssignment::of)
        .ok_or_else(|| ApiError::internal("assignment vanished after insert"))?;
    Ok((StatusCode::CREATED, Json(saved)))
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct ReasonQuery {
    /// 写入审计日志。
    pub reason: String,
}

/// 撤销一条任命（免去管理员或版主）。
#[utoipa::path(
    delete, path = "/api/v1/admin/staff/{id}", tag = "admin",
    params(("id" = i64, Path), ReasonQuery),
    responses(
        (status = 204),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody)
    )
)]
pub async fn remove_staff(
    State(state): State<AppState>,
    admin: AdminSession,
    Path(id): Path<i64>,
    Query(q): Query<ReasonQuery>,
) -> ApiResult<StatusCode> {
    sensitive(&admin, Permission::ManageStaff)?;
    let reason = moderation::reason(&q.reason)?;
    let mut tx = state.db.begin().await?;
    let removed = staff::revoke(&mut *tx, id)
        .await?
        .ok_or(ApiError(ErrorCode::NotFound))?;
    audit::append(
        &mut *tx,
        admin.user_id,
        "staff.remove",
        Some(&format!("user:{}", removed.user_id)),
        Some(reason),
        &json!({ "assignment_id": id, "role": Role::from_id(removed.role), "forum_id": removed.forum_id }),
    )
    .await?;
    outbox::staff_alert(
        &mut *tx,
        "staff.remove",
        &format!("站长免去用户 #{} 的任命 #{id}：{reason}", removed.user_id),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ModerationSettingsView {
    pub settings: ModerationSettings,
    /// 每项允许的最小值与最大值。
    pub min: ModerationSettings,
    pub max: ModerationSettings,
    pub defaults: ModerationSettings,
}

/// 管理操作限速与异常检测阈值。
#[utoipa::path(
    get, path = "/api/v1/admin/settings/moderation", tag = "admin",
    responses((status = 200, body = ModerationSettingsView), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody))
)]
pub async fn get_settings(
    State(state): State<AppState>,
    admin: AdminSession,
) -> ApiResult<Json<ModerationSettingsView>> {
    admin.require(Permission::ManageSettings)?;
    let settings = forum_db::settings::moderation(&state.db).await?;
    let (min, max) = ModerationSettings::bounds();
    Ok(Json(ModerationSettingsView {
        settings,
        min,
        max,
        defaults: ModerationSettings::default(),
    }))
}

/// 修改阈值（必须在上下限内）。
#[utoipa::path(
    put, path = "/api/v1/admin/settings/moderation", tag = "admin",
    request_body = ModerationSettings,
    responses(
        (status = 200, body = ModerationSettingsView),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody)
    )
)]
pub async fn put_settings(
    State(state): State<AppState>,
    admin: AdminSession,
    Json(req): Json<ModerationSettings>,
) -> ApiResult<Json<ModerationSettingsView>> {
    sensitive(&admin, Permission::ManageSettings)?;
    if !req.is_valid() {
        return Err(ApiError(ErrorCode::InvalidParameter));
    }
    let mut tx = state.db.begin().await?;
    let old = forum_db::settings::moderation(&mut *tx).await?;
    forum_db::settings::set_moderation(&mut *tx, &req, admin.user_id).await?;
    audit::append(
        &mut *tx,
        admin.user_id,
        "settings.moderation",
        Some("settings:moderation"),
        None,
        &json!({ "old": old, "new": req }),
    )
    .await?;
    outbox::staff_alert(
        &mut *tx,
        "settings.moderation",
        &format!(
            "站长修改了管理限速与异常阈值：{}",
            serde_json::to_string(&req).map_err(ApiError::internal)?
        ),
    )
    .await?;
    tx.commit().await?;
    let (min, max) = ModerationSettings::bounds();
    Ok(Json(ModerationSettingsView {
        settings: req,
        min,
        max,
        defaults: ModerationSettings::default(),
    }))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SuspensionView {
    pub id: i64,
    pub user_id: i64,
    pub username: Option<String>,
    /// `anomaly`：异常检测自动暂停。
    pub reason: String,
    #[schema(value_type = Object)]
    pub detail: Value,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SuspensionList {
    pub suspensions: Vec<SuspensionView>,
}

/// 管理权限被暂停中的账号。
#[utoipa::path(
    get, path = "/api/v1/admin/suspensions", tag = "admin",
    responses((status = 200, body = SuspensionList), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody))
)]
pub async fn list_suspensions(
    State(state): State<AppState>,
    admin: AdminSession,
) -> ApiResult<Json<SuspensionList>> {
    admin.require(Permission::Reinstate)?;
    let rows = staff::suspensions(&state.db).await?;
    Ok(Json(SuspensionList {
        suspensions: rows
            .into_iter()
            .map(|s| SuspensionView {
                id: s.id,
                user_id: s.user_id,
                username: s.username,
                reason: s.reason,
                detail: s.detail,
                created_at: s.created_at,
            })
            .collect(),
    }))
}

/// 恢复被暂停的管理权限（站长确认后）。
#[utoipa::path(
    post, path = "/api/v1/admin/suspensions/{id}/reinstate", tag = "admin",
    params(("id" = i64, Path), ReasonQuery),
    responses(
        (status = 204),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody)
    )
)]
pub async fn reinstate(
    State(state): State<AppState>,
    admin: AdminSession,
    Path(id): Path<i64>,
    Query(q): Query<ReasonQuery>,
) -> ApiResult<StatusCode> {
    sensitive(&admin, Permission::Reinstate)?;
    let reason = moderation::reason(&q.reason)?;
    let mut tx = state.db.begin().await?;
    let user_id = staff::reinstate(&mut *tx, id, admin.user_id)
        .await?
        .ok_or(ApiError(ErrorCode::NotFound))?;
    audit::append(
        &mut *tx,
        admin.user_id,
        "staff.reinstate",
        Some(&format!("user:{user_id}")),
        Some(reason),
        &json!({ "suspension_id": id }),
    )
    .await?;
    outbox::staff_alert(
        &mut *tx,
        "staff.reinstate",
        &format!("站长恢复了账号 #{user_id} 的管理权限：{reason}"),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RevokeRequest {
    /// 要撤销其管理操作的账号。
    pub actor_id: i64,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub from: OffsetDateTime,
    /// 不含；与 `from` 最多相隔 31 天。
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub to: OffsetDateTime,
    /// 写入审计日志。
    pub reason: String,
    /// `false`：只预览（在回滚的事务里试做一遍）；`true`：执行（需 5 分钟内再验证）。
    pub execute: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RevokeItem {
    pub log_id: i64,
    pub action: String,
    pub target: Option<String>,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// 预览时 `reverted` 表示可以撤销。
    pub status: RevertStatus,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RevokeResult {
    pub executed: bool,
    /// 新的在前（撤销也按这个顺序做）。
    pub items: Vec<RevokeItem>,
    pub reverted: usize,
    /// 超过 500 条只处理最新的 500 条，需要缩小时间段再做一次。
    pub truncated: bool,
}

/// 一键撤销某账号在某时间段内的全部管理操作（ADR 0013 第 6 条）：先预览，再执行。
/// 只还原仍是该操作结果的状态，之后又被改过的跳过；执行本身写审计日志并推送管理员群。
#[utoipa::path(
    post, path = "/api/v1/admin/revoke", tag = "admin",
    request_body = RevokeRequest,
    responses(
        (status = 200, body = RevokeResult),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody)
    )
)]
pub async fn revoke(
    State(state): State<AppState>,
    admin: AdminSession,
    Json(req): Json<RevokeRequest>,
) -> ApiResult<Json<RevokeResult>> {
    if req.execute {
        sensitive(&admin, Permission::RevokeActions)?;
    } else {
        admin.require(Permission::RevokeActions)?;
    }
    let reason = moderation::reason(&req.reason)?;
    if req.to <= req.from || req.to - req.from > REVOKE_MAX_SPAN {
        return Err(ApiError(ErrorCode::InvalidParameter));
    }
    let mut tx = state.db.begin().await?;
    let mut entries =
        audit::by_actor(&mut *tx, req.actor_id, req.from, req.to, REVOKE_LIMIT + 1).await?;
    let truncated = entries.len() as i64 > REVOKE_LIMIT;
    entries.truncate(REVOKE_LIMIT as usize);
    let mut effects = RevertEffects::default();
    let mut items = Vec::with_capacity(entries.len());
    let mut reverted = Vec::new();
    for e in &entries {
        let mut status =
            moderation::revert(&state, &mut tx, admin.user_id, e, &mut effects).await?;
        if status == RevertStatus::Reverted {
            let detail = json!({ "actor_id": req.actor_id, "batch": true });
            if audit::append_revert(&mut *tx, admin.user_id, e, reason, &detail)
                .await?
                .is_some()
            {
                reverted.push(e.id);
            } else {
                status = RevertStatus::AlreadyReverted;
            }
        }
        items.push(RevokeItem {
            log_id: e.id,
            action: e.action.clone(),
            target: e.target.clone(),
            created_at: e.created_at,
            status,
        });
    }
    if req.execute {
        audit::append(
            &mut *tx,
            admin.user_id,
            "admin.revoke_actions",
            Some(&format!("user:{}", req.actor_id)),
            Some(reason),
            &json!({
                "from": req.from.unix_timestamp(), "to": req.to.unix_timestamp(),
                "reverted": reverted, "truncated": truncated,
            }),
        )
        .await?;
        outbox::staff_alert(
            &mut *tx,
            "admin.revoke_actions",
            &format!(
                "站长撤销了账号 #{} 的 {} 条管理操作：{reason}",
                req.actor_id,
                reverted.len()
            ),
        )
        .await?;
        tx.commit().await?;
        effects.apply(&state).await;
    } else {
        tx.rollback().await?;
    }
    Ok(Json(RevokeResult {
        executed: req.execute,
        reverted: reverted.len(),
        items,
        truncated,
    }))
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct AuditQuery {
    /// 只看这个账号的操作。
    pub actor_id: Option<i64>,
    /// 游标：上一页的 `next_cursor`。
    pub cursor: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuditEntryView {
    pub id: i64,
    pub actor_id: i64,
    pub action: String,
    pub target: Option<String>,
    pub reason: Option<String>,
    #[schema(value_type = Object)]
    pub detail: Value,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// 撤销记录指向被撤销的日志。
    pub reverts: Option<i64>,
    pub reverted: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuditPage {
    pub entries: Vec<AuditEntryView>,
    pub next_cursor: Option<i64>,
}

/// 只增审计日志，新的在前。
#[utoipa::path(
    get, path = "/api/v1/admin/audit", tag = "admin",
    params(AuditQuery),
    responses((status = 200, body = AuditPage), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody))
)]
pub async fn audit_log(
    State(state): State<AppState>,
    admin: AdminSession,
    Query(q): Query<AuditQuery>,
) -> ApiResult<Json<AuditPage>> {
    admin.require(Permission::RevokeActions)?;
    let rows = audit::list(&state.db, q.actor_id, q.cursor, AUDIT_PAGE).await?;
    let next_cursor = (rows.len() as i64 == AUDIT_PAGE)
        .then(|| rows.last().map(|r| r.id))
        .flatten();
    Ok(Json(AuditPage {
        entries: rows
            .into_iter()
            .map(|e| AuditEntryView {
                id: e.id,
                actor_id: e.actor_id,
                action: e.action,
                target: e.target,
                reason: e.reason,
                detail: e.detail,
                created_at: e.created_at,
                reverts: e.reverts,
                reverted: e.reverted,
            })
            .collect(),
        next_cursor,
    }))
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct UserSearchQuery {
    /// 用户名前缀（不区分大小写）。
    pub q: String,
}

/// 任命时按用户名找人（后台页面不能跨源调论坛接口，这里给一份同样的结果）。
#[utoipa::path(
    get, path = "/api/v1/admin/users/search", tag = "admin",
    params(UserSearchQuery),
    responses(
        (status = 200, body = Vec<MentionView>), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody)
    )
)]
pub async fn admin_search_users(
    State(state): State<AppState>,
    admin: AdminSession,
    Query(q): Query<UserSearchQuery>,
) -> ApiResult<Json<Vec<MentionView>>> {
    admin.require(Permission::ManageStaff)?;
    Ok(Json(search_prefix(&state, &q.q).await?))
}

/// 全部板块（任命版主时选板块；含等级不够看不到的）。
#[utoipa::path(
    get, path = "/api/v1/admin/forums", tag = "admin",
    responses((status = 200, body = Vec<ForumView>), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody))
)]
pub async fn admin_forums(
    State(state): State<AppState>,
    admin: AdminSession,
) -> ApiResult<Json<Vec<ForumView>>> {
    admin.require(Permission::ManageStaff)?;
    let forums = forum_db::forums::list_all(&state.db).await?;
    Ok(Json(forums.iter().map(ForumView::from).collect()))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AttentionCase {
    pub case_id: i64,
    pub order_id: i64,
    /// `reviewing` / `owner_review` / `appeal_window` / `appealed` / `ready` / `halted`。
    pub status: String,
    pub panel_size: i16,
    /// 订单金额（最小单位）与代币精度。
    pub amount: String,
    pub token_decimals: i16,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub evidence_until: OffsetDateTime,
    #[schema(value_type = Option<String>, format = DateTime)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub appeal_until: Option<OffsetDateTime>,
    /// `resolveTimeout` 可调用的时间。
    #[schema(value_type = Option<String>, format = DateTime)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub timeout_at: Option<OffsetDateTime>,
    /// 剩余不足 3 天（或已可调用）。
    pub timeout_soon: bool,
    /// 缺人：裁决中在岗的仲裁员不够，或申诉后还没有复核人（全部回避或已参与时需要站长复核）。
    pub unstaffed: bool,
    pub halt_reason: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ArbitrationOverview {
    pub workload: Workload,
    /// 站长要处理或盯着的案件（待终裁、待复核、已叫停、待执行、缺人、临近超时兜底）。
    pub cases: Vec<AttentionCase>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct OverviewQuery {
    /// 工作量统计最近多少天（1–366，默认 30）。
    pub days: Option<i64>,
}

/// 仲裁总览（后台）：工作量与按时率、需要站长关注的案件。裁决本身仍在论坛仲裁页用裁决 Passkey 做。
#[utoipa::path(
    get, path = "/api/v1/admin/arbitration", tag = "admin",
    params(OverviewQuery),
    responses(
        (status = 200, body = ArbitrationOverview), (status = 400, body = ErrorBody),
        (status = 401, body = ErrorBody)
    )
)]
pub async fn arbitration_overview(
    State(state): State<AppState>,
    // 后台会话本身就确认了是站长
    _admin: AdminSession,
    Query(q): Query<OverviewQuery>,
) -> ApiResult<Json<ArbitrationOverview>> {
    let workload = arbitration::workload(&state, q.days).await?;
    let now = OffsetDateTime::now_utc();
    let mut cases = Vec::new();
    for (id, _) in arb::owner_queue(&state.db).await? {
        let Some(c) = arb::case_by_id(&state.db, id).await? else {
            continue;
        };
        let active = arb::assignments(&state.db, id)
            .await?
            .into_iter()
            .filter(|a| a.is_active())
            .collect::<Vec<_>>();
        let seated = |role: &str| active.iter().filter(|a| a.role == role).count();
        let unstaffed = match c.status.as_str() {
            arb::STATUS_REVIEWING => seated(arb::ROLE_PANEL) < c.panel(),
            arb::STATUS_APPEALED => seated(arb::ROLE_APPEAL) == 0,
            _ => false,
        };
        let timeout_at = c.timeout_at();
        cases.push(AttentionCase {
            case_id: c.id,
            order_id: c.order_id,
            panel_size: c.panel_size,
            amount: c.amount.clone(),
            token_decimals: c.token_decimals,
            evidence_until: c.evidence_until,
            appeal_until: c.appeal_until,
            timeout_soon: timeout_at
                .is_some_and(|t| t - now < forum_core::arbitration::TIMEOUT_ALERT),
            timeout_at,
            unstaffed,
            halt_reason: c
                .halt_reason
                .clone()
                .filter(|_| c.status == arb::STATUS_HALTED),
            status: c.status,
        });
    }
    Ok(Json(ArbitrationOverview { workload, cases }))
}

/// 全部可执行裁决的 Safe 批量交易文件（后台下载，与论坛 `GET /api/v1/arbitration/execution` 相同）。
#[utoipa::path(
    get, path = "/api/v1/admin/arbitration/execution", tag = "admin",
    responses((status = 200, body = [ExecutionBatch]), (status = 401, body = ErrorBody))
)]
pub async fn arbitration_execution(
    State(state): State<AppState>,
    _admin: AdminSession,
) -> ApiResult<Json<Vec<ExecutionBatch>>> {
    Ok(Json(arbitration::ready_batches(&state).await?))
}
