//! 站长后台的发件箱死信（第 9 轮评审）：按类型的待发与死信条数、死信列表、重新投递与丢弃。只有站长（后台会话）；
//! 不涉及资金，不要求再验证。处理动作写审计日志。只读得到类型、次数与错误，读不到 payload（验证码、邮箱地址等）。

use axum::Json;
use axum::extract::{Query, State};
use forum_core::ErrorCode;
use forum_core::staff::Permission;
use forum_db::{audit, outbox};
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::OffsetDateTime;
use utoipa::{IntoParams, ToSchema};

use crate::admin::AdminSession;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::state::AppState;

const DEAD_PAGE: i64 = 50;
/// 一次最多处理的死信条数。
const MAX_IDS: usize = 100;

#[derive(Debug, Serialize, ToSchema)]
pub struct OutboxKindCounts {
    pub kind: String,
    /// 待发（含退避中与投递中）。
    pub pending: i64,
    /// 其中失败过、正在退避的。
    pub retrying: i64,
    pub dead: i64,
    /// 最早一条待发消息的创建时间。
    #[schema(value_type = Option<String>, format = DateTime)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub oldest_pending: Option<OffsetDateTime>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DeadLetterView {
    pub id: i64,
    pub kind: String,
    pub attempts: i32,
    /// 最后一次的错误（已去掉邮箱地址等）。
    pub last_error: Option<String>,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub failed_at: OffsetDateTime,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OutboxOverview {
    pub kinds: Vec<OutboxKindCounts>,
    /// 死信，新的在前，每页 50 条。
    pub dead: Vec<DeadLetterView>,
    pub next_cursor: Option<i64>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct OutboxQuery {
    /// 只看这一种消息的死信。
    pub kind: Option<String>,
    /// 上一页的 `next_cursor`。
    pub cursor: Option<i64>,
}

/// 发件箱概览：每种消息的待发与死信条数，以及死信列表。
#[utoipa::path(
    get, path = "/api/v1/admin/outbox", tag = "admin",
    params(OutboxQuery),
    responses((status = 200, body = OutboxOverview), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody))
)]
pub async fn admin_outbox(
    State(state): State<AppState>,
    admin: AdminSession,
    Query(q): Query<OutboxQuery>,
) -> ApiResult<Json<OutboxOverview>> {
    admin.require(Permission::ManageSettings)?;
    let kinds = outbox::counts(&state.db).await?;
    let dead = outbox::dead_letters(&state.db, q.kind.as_deref(), q.cursor, DEAD_PAGE).await?;
    let next_cursor = (dead.len() as i64 == DEAD_PAGE)
        .then(|| dead.last().map(|d| d.id))
        .flatten();
    Ok(Json(OutboxOverview {
        kinds: kinds
            .into_iter()
            .map(|c| OutboxKindCounts {
                kind: c.kind,
                pending: c.pending,
                retrying: c.retrying,
                dead: c.dead,
                oldest_pending: c.oldest_pending,
            })
            .collect(),
        dead: dead
            .into_iter()
            .map(|d| DeadLetterView {
                id: d.id,
                kind: d.kind,
                attempts: d.attempts,
                last_error: d.last_error,
                created_at: d.created_at,
                failed_at: d.failed_at,
            })
            .collect(),
        next_cursor,
    }))
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DeadLetterAction {
    /// 1–100 条死信的 ID。
    pub ids: Vec<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DeadLetterResult {
    /// 实际处理的 ID（已不是死信的忽略）。
    pub ids: Vec<i64>,
}

#[derive(Clone, Copy)]
enum Action {
    Requeue,
    Discard,
}

/// 处理一批死信并写审计日志（`outbox.requeue` / `outbox.discard`，详情为处理的 ID 与类型）。
async fn handle(
    state: &AppState,
    admin: &AdminSession,
    req: &DeadLetterAction,
    action: Action,
) -> ApiResult<Json<DeadLetterResult>> {
    admin.require(Permission::ManageSettings)?;
    if req.ids.is_empty() || req.ids.len() > MAX_IDS {
        return Err(ApiError(ErrorCode::InvalidParameter));
    }
    let mut tx = state.db.begin().await?;
    let (handled, name) = match action {
        Action::Requeue => (outbox::requeue(&mut *tx, &req.ids).await?, "outbox.requeue"),
        Action::Discard => (outbox::discard(&mut *tx, &req.ids).await?, "outbox.discard"),
    };
    let ids: Vec<i64> = handled.iter().map(|h| h.id).collect();
    if !ids.is_empty() {
        let kinds: Vec<&str> = handled.iter().map(|h| h.kind.as_str()).collect();
        audit::append(
            &mut *tx,
            admin.user_id,
            name,
            None,
            None,
            &json!({ "ids": ids, "kinds": kinds }),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(Json(DeadLetterResult { ids }))
}

/// 重新投递死信：尝试次数清零、立即到期，payload 原样。修好导致失败的原因（约束、配置）之后再用。
#[utoipa::path(
    post, path = "/api/v1/admin/outbox/requeue", tag = "admin",
    request_body = DeadLetterAction,
    responses(
        (status = 200, body = DeadLetterResult),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody)
    )
)]
pub async fn admin_outbox_requeue(
    State(state): State<AppState>,
    admin: AdminSession,
    Json(req): Json<DeadLetterAction>,
) -> ApiResult<Json<DeadLetterResult>> {
    handle(&state, &admin, &req, Action::Requeue).await
}

/// 丢弃死信：清空 payload，不能再重新投递。
#[utoipa::path(
    post, path = "/api/v1/admin/outbox/discard", tag = "admin",
    request_body = DeadLetterAction,
    responses(
        (status = 200, body = DeadLetterResult),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody)
    )
)]
pub async fn admin_outbox_discard(
    State(state): State<AppState>,
    admin: AdminSession,
    Json(req): Json<DeadLetterAction>,
) -> ApiResult<Json<DeadLetterResult>> {
    handle(&state, &admin, &req, Action::Discard).await
}
