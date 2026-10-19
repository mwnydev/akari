use axum::Json;
use axum::extract::State;
use forum_core::ErrorCode;
use forum_db::uploads::UploadStatus;
use forum_db::users::SetUsername;
use serde::{Deserialize, Serialize};
use tower_sessions::Session;
use utoipa::ToSchema;

use super::SESSION_USER_ID;
use crate::cards;
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

#[derive(Debug, Deserialize, ToSchema)]
pub struct UsernameRequest {
    pub username: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UsernameSet {
    pub username: String,
}

/// 设置或修改用户名：2–20 个字（汉字、字母、数字、下划线），不区分大小写唯一，首次设置后 30 天才能再改。
#[utoipa::path(
    put, path = "/api/v1/me/username", tag = "auth",
    request_body = UsernameRequest,
    responses(
        (status = 200, body = UsernameSet), (status = 400, body = ErrorBody), (status = 401, body = ErrorBody),
        (status = 409, body = ErrorBody)
    )
)]
pub async fn set_username(
    State(state): State<AppState>,
    session: Session,
    Json(req): Json<UsernameRequest>,
) -> ApiResult<Json<UsernameSet>> {
    let id = current_user_id(&session).await?;
    let name = forum_core::username::validate(&req.username)?;
    match forum_db::users::set_username(&state.db, id, &name).await? {
        SetUsername::Done => {}
        SetUsername::TooSoon => return Err(ApiError(ErrorCode::UsernameChangeTooSoon)),
        SetUsername::Taken => return Err(ApiError(ErrorCode::UsernameTaken)),
    }
    cards::invalidate(&state, &[id]).await;
    Ok(Json(UsernameSet { username: name }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AvatarRequest {
    /// 已处理完成的本人上传（`POST /api/v1/uploads`）。
    pub upload_id: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AvatarSet {
    pub avatar_url: String,
}

/// 用一张处理完成的上传图片做头像（取最小的 320 尺寸）；还没处理完返回 `INVALID_PARAMETER`。
#[utoipa::path(
    put, path = "/api/v1/me/avatar", tag = "auth",
    request_body = AvatarRequest,
    responses(
        (status = 200, body = AvatarSet), (status = 400, body = ErrorBody), (status = 401, body = ErrorBody),
        (status = 404, body = ErrorBody)
    )
)]
pub async fn set_avatar(
    State(state): State<AppState>,
    session: Session,
    Json(req): Json<AvatarRequest>,
) -> ApiResult<Json<AvatarSet>> {
    let id = current_user_id(&session).await?;
    let upload = forum_db::uploads::by_id(&state.db, req.upload_id)
        .await?
        .filter(|u| u.user_id == id)
        .ok_or(ApiError(ErrorCode::NotFound))?;
    if upload.status != UploadStatus::Done {
        return Err(ApiError(ErrorCode::InvalidParameter));
    }
    let url = upload
        .variants
        .as_ref()
        .and_then(|v| v.get(0)?.get("url")?.as_str().map(str::to_owned))
        .ok_or_else(|| ApiError::internal("finished upload without variants"))?;
    forum_db::users::set_avatar(&state.db, id, &url).await?;
    cards::invalidate(&state, &[id]).await;
    Ok(Json(AvatarSet { avatar_url: url }))
}
