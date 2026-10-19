//! 站内通知（plan 第 4 节）：回复、@、私信、报价、订单、系统。由 worker 从发件箱写入并推送 `notification` 事件；
//! 这里只读和标记已读。只显示 180 天内的通知。

use axum::Json;
use axum::extract::{Query, State};
use forum_core::ErrorCode;
use forum_core::chat::Notification;
use forum_db::notifications as db;
use serde::{Deserialize, Serialize};
use tower_sessions::Session;
use utoipa::{IntoParams, ToSchema};

use super::me::current_user_id;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::state::AppState;

/// `before` 取它之前的一页（新的在前；都不给取最新一页），`after` 取它之后的一页（按时间正序，重连补拉）。
#[derive(Debug, Deserialize, IntoParams)]
pub struct NotificationsQuery {
    pub before: Option<i64>,
    pub after: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct NotificationPage {
    pub notifications: Vec<Notification>,
    /// 翻页方向上还有更多时，下一页的游标（`before` / `after` 用）。
    pub next_cursor: Option<i64>,
    /// 已读水位：ID 不大于它的都已读。
    pub last_read_id: i64,
}

/// 我的通知，每页 20 条。
#[utoipa::path(
    get, path = "/api/v1/notifications", tag = "notifications", operation_id = "list_notifications",
    params(NotificationsQuery),
    responses((status = 200, body = NotificationPage), (status = 400, body = ErrorBody), (status = 401, body = ErrorBody))
)]
pub async fn list(
    State(state): State<AppState>,
    session: Session,
    Query(q): Query<NotificationsQuery>,
) -> ApiResult<Json<NotificationPage>> {
    let me = current_user_id(&session).await?;
    let mark = db::read_mark(&state.db, me).await?;
    let rows = match (q.before, q.after) {
        (Some(_), Some(_)) => return Err(ApiError(ErrorCode::InvalidParameter)),
        (_, Some(a)) => db::after(&state.db, me, a, db::PAGE, mark).await?,
        (b, None) => db::before(&state.db, me, b.unwrap_or(i64::MAX), db::PAGE, mark).await?,
    };
    let next_cursor = rows
        .last()
        .map(|n| n.id)
        .filter(|_| i64::try_from(rows.len()) == Ok(db::PAGE));
    Ok(Json(NotificationPage {
        notifications: rows,
        next_cursor,
        last_read_id: mark,
    }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ReadNotifications {
    /// 这条及之前的通知全部标为已读。
    pub up_to_id: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UnreadCount {
    /// 未读数，最多数到 100（界面显示 99+）。
    pub count: i64,
    pub last_read_id: i64,
}

/// 标记已读（水位只前进），返回剩余未读数。
#[utoipa::path(
    post, path = "/api/v1/notifications/read", tag = "notifications", operation_id = "read_notifications",
    request_body = ReadNotifications,
    responses((status = 200, body = UnreadCount), (status = 401, body = ErrorBody))
)]
pub async fn read(
    State(state): State<AppState>,
    session: Session,
    Json(req): Json<ReadNotifications>,
) -> ApiResult<Json<UnreadCount>> {
    let me = current_user_id(&session).await?;
    let mark = db::mark_read(&state.db, me, req.up_to_id).await?;
    Ok(Json(UnreadCount {
        count: db::unread(&state.db, me, mark).await?,
        last_read_id: mark,
    }))
}

#[utoipa::path(
    get, path = "/api/v1/notifications/unread", tag = "notifications", operation_id = "unread_notifications",
    responses((status = 200, body = UnreadCount), (status = 401, body = ErrorBody))
)]
pub async fn unread(
    State(state): State<AppState>,
    session: Session,
) -> ApiResult<Json<UnreadCount>> {
    let me = current_user_id(&session).await?;
    let mark = db::read_mark(&state.db, me).await?;
    Ok(Json(UnreadCount {
        count: db::unread(&state.db, me, mark).await?,
        last_read_id: mark,
    }))
}
