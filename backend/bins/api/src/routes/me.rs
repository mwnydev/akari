use axum::Json;
use axum::extract::State;
use forum_core::ErrorCode;
use forum_core::staff::{DurationLimit, Permission, Role, Scope, StaffRoles};
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
    /// 管理角色与权限；普通用户为空。论坛界面据此显示管理功能（服务端每个管理接口仍会再校验）。
    pub staff: Option<StaffView>,
}

impl Me {
    pub fn new(u: forum_db::users::User, roles: &StaffRoles) -> Self {
        Self {
            id: u.id,
            email: u.email,
            username: u.username,
            staff: roles.is_staff().then(|| StaffView::from(roles)),
        }
    }
}

/// 管理角色（ADR 0013）：站长、管理员、版主。
#[derive(Debug, Serialize, ToSchema)]
pub struct StaffView {
    pub roles: Vec<Role>,
    /// 作为版主负责的板块。
    pub boards: Vec<i32>,
    /// 管理权限被自动暂停（异常检测），待站长恢复；此时下面两个权限列表为空。
    pub suspended: bool,
    /// 全站范围可用的论坛管理权限（管理员、站长）。
    pub site_permissions: Vec<Permission>,
    /// 只在 `boards` 内可用的权限（版主）。
    pub board_permissions: Vec<Permission>,
    /// 禁言最长秒数；为空表示不限（可永久）。没有禁言权限时看权限列表。
    pub max_mute_secs: Option<i64>,
    /// 禁拍最长秒数；为空表示不限。
    pub max_auction_ban_secs: Option<i64>,
    /// 能进后台（只有站长，`administrator` 子域名，需后台 Passkey）。
    pub admin_backend: bool,
}

impl From<&StaffRoles> for StaffView {
    fn from(r: &StaffRoles) -> Self {
        // 管理员与站长全站不限；版主在自己板块内有上限（各板块相同）。
        let scope = match r.boards.first() {
            Some(b) if !r.owner && !r.admin => Scope::Board(*b),
            _ => Scope::Site,
        };
        let cap = |p| match r.duration_limit(p, scope) {
            Some(DurationLimit::Upto(s)) => Some(s),
            _ => None,
        };
        Self {
            roles: r.roles(),
            boards: r.boards.clone(),
            suspended: r.suspended,
            site_permissions: r.site_permissions(),
            board_permissions: r.board_permissions(),
            max_mute_secs: cap(Permission::Mute),
            max_auction_ban_secs: cap(Permission::AuctionBan),
            admin_backend: r.owner,
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

/// 当前登录用户，含管理角色与权限。
#[utoipa::path(
    get, path = "/api/v1/me", tag = "auth",
    responses((status = 200, body = Me), (status = 401, body = ErrorBody))
)]
pub async fn me(State(state): State<AppState>, session: Session) -> ApiResult<Json<Me>> {
    let id = current_user_id(&session).await?;
    let user = forum_db::users::find_by_id(&state.db, id).await?;
    match user {
        Some(u) => {
            let roles = forum_db::staff::roles(&state.db, id).await?;
            Ok(Json(Me::new(u, &roles)))
        }
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
