use axum::Json;
use axum::extract::State;
use forum_core::ErrorCode;
use serde::Serialize;
use tower_sessions::Session;
use utoipa::ToSchema;

use super::SESSION_USER_ID;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::state::AppState;

#[derive(Debug, Serialize, ToSchema)]
pub struct Me {
    pub id: i64,
    pub email: String,
    pub username: Option<String>,
}

impl From<forum_db::users::User> for Me {
    fn from(u: forum_db::users::User) -> Self {
        Self {
            id: u.id,
            email: u.email,
            username: u.username,
        }
    }
}

/// 从会话取当前用户 ID；未登录返回 `UNAUTHORIZED`。
pub async fn current_user_id(session: &Session) -> ApiResult<i64> {
    session
        .get::<i64>(SESSION_USER_ID)
        .await?
        .ok_or(ApiError(ErrorCode::Unauthorized))
}

/// 当前登录用户。
#[utoipa::path(
    get, path = "/api/v1/me", tag = "auth",
    responses((status = 200, body = Me), (status = 401, body = ErrorBody))
)]
pub async fn me(State(state): State<AppState>, session: Session) -> ApiResult<Json<Me>> {
    let id = current_user_id(&session).await?;
    let user = forum_db::users::find_by_id(&state.db, id).await?;
    match user {
        Some(u) => Ok(Json(u.into())),
        // 账号被删除但会话还在：清掉会话按未登录处理。
        None => {
            session.flush().await?;
            Err(ApiError(ErrorCode::Unauthorized))
        }
    }
}
