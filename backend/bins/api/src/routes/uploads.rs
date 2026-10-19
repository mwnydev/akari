//! 图片上传（plan 第 12 节）：申请预签名地址 → 浏览器直传存储桶临时目录 → 确认 → worker 转 WebP。

use std::collections::BTreeMap;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use forum_core::ErrorCode;
use forum_db::uploads::{self, Upload, UploadStatus};
use forum_kv::SlidingWindow;
use forum_media::{Format, MAX_UPLOAD_BYTES};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tower_sessions::Session;
use utoipa::ToSchema;

use super::me::current_user_id;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::posting;
use crate::state::AppState;

/// 每人每小时最多申请多少次上传（初始值，按实际使用调整）。
const UPLOAD_RATE: SlidingWindow = SlidingWindow::new("upload", 60, Duration::from_secs(3600));

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewUploadRequest {
    /// `image/jpeg`、`image/png`、`image/gif`、`image/webp`、`image/heic`（`image/heif`）。
    pub content_type: String,
    /// 字节数，≤ 10 MB。上传时的 `Content-Length` 必须与它一致。
    pub size: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct NewUpload {
    pub id: i64,
    /// 用 `PUT` 把文件原样上传到这里，带上 `headers` 里的每个请求头。
    pub upload_url: String,
    pub headers: BTreeMap<String, String>,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum UploadState {
    /// 等待浏览器上传并确认。
    Pending,
    /// 处理中（含排队）。
    Processing,
    Done,
    Failed,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ImageVariant {
    pub width: u32,
    pub height: u32,
    pub url: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UploadView {
    pub id: i64,
    pub status: UploadState,
    /// 处理后（按 EXIF 方向转正）的尺寸。
    pub width: Option<i32>,
    pub height: Option<i32>,
    /// 最长边 320 / 1280 / 2560 的 WebP（不放大，原图小时更少），从小到大。
    pub variants: Vec<ImageVariant>,
    /// 失败原因：`unsupported`、`too_large`、`corrupt`、`missing`。
    pub error: Option<String>,
}

impl TryFrom<Upload> for UploadView {
    type Error = ApiError;

    fn try_from(u: Upload) -> Result<Self, ApiError> {
        let variants = match u.variants {
            Some(v) => serde_json::from_value(v).map_err(ApiError::internal)?,
            None => Vec::new(),
        };
        Ok(Self {
            id: u.id,
            status: match u.status {
                UploadStatus::Pending => UploadState::Pending,
                UploadStatus::Queued | UploadStatus::Processing => UploadState::Processing,
                UploadStatus::Done => UploadState::Done,
                UploadStatus::Failed => UploadState::Failed,
            },
            width: u.width,
            height: u.height,
            variants,
            error: (u.status == UploadStatus::Failed)
                .then_some(u.error)
                .flatten(),
        })
    }
}

/// 申请一次图片上传：返回直传临时目录的预签名 `PUT` 地址（10 分钟有效，类型与字节数写在签名里）。
#[utoipa::path(
    post, path = "/api/v1/uploads", tag = "uploads",
    request_body = NewUploadRequest,
    responses(
        (status = 201, body = NewUpload), (status = 400, body = ErrorBody), (status = 401, body = ErrorBody),
        (status = 413, body = ErrorBody), (status = 429, body = ErrorBody), (status = 503, body = ErrorBody)
    )
)]
pub async fn create_upload(
    State(state): State<AppState>,
    session: Session,
    Json(req): Json<NewUploadRequest>,
) -> ApiResult<(StatusCode, Json<NewUpload>)> {
    let user_id = current_user_id(&session).await?;
    let storage = state
        .media
        .clone()
        .ok_or(ApiError(ErrorCode::UploadsUnavailable))?;
    let format =
        Format::from_mime(&req.content_type).ok_or(ApiError(ErrorCode::UnsupportedImage))?;
    if req.size == 0 {
        return Err(ApiError(ErrorCode::InvalidParameter));
    }
    if req.size > MAX_UPLOAD_BYTES {
        return Err(ApiError(ErrorCode::ImageTooLarge));
    }
    let size = i32::try_from(req.size).map_err(|_| ApiError(ErrorCode::ImageTooLarge))?;
    posting::limit(&state, &UPLOAD_RATE, user_id).await?;
    let key = forum_media::tmp_key(user_id);
    let id = uploads::create(&state.db, user_id, format.mime(), size, &key).await?;
    let signed = storage.presign_upload(&key, format.mime(), req.size);
    Ok((
        StatusCode::CREATED,
        Json(NewUpload {
            id,
            upload_url: signed.url,
            headers: signed
                .headers
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v))
                .collect(),
            expires_at: OffsetDateTime::now_utc() + forum_media::storage::UPLOAD_URL_TTL,
        }),
    ))
}

/// 本人的上传；别人的当作不存在。
async fn own(state: &AppState, session: &Session, id: i64) -> ApiResult<Upload> {
    let user_id = current_user_id(session).await?;
    uploads::by_id(&state.db, id)
        .await?
        .filter(|u| u.user_id == user_id)
        .ok_or(ApiError(ErrorCode::NotFound))
}

/// 浏览器上传完成后确认，排队转换；重复确认返回当前状态。
#[utoipa::path(
    post, path = "/api/v1/uploads/{id}/complete", tag = "uploads",
    params(("id" = i64, Path)),
    responses((status = 202, body = UploadView), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody))
)]
pub async fn complete_upload(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
) -> ApiResult<(StatusCode, Json<UploadView>)> {
    let upload = own(&state, &session, id).await?;
    uploads::queue(&state.db, id, upload.user_id).await?;
    let now = uploads::by_id(&state.db, id)
        .await?
        .ok_or(ApiError(ErrorCode::NotFound))?;
    Ok((StatusCode::ACCEPTED, Json(now.try_into()?)))
}

/// 上传状态与处理结果（处理中前端显示占位，通常 1 秒内完成）。
#[utoipa::path(
    get, path = "/api/v1/uploads/{id}", tag = "uploads",
    params(("id" = i64, Path)),
    responses((status = 200, body = UploadView), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody))
)]
pub async fn get_upload(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
) -> ApiResult<Json<UploadView>> {
    Ok(Json(own(&state, &session, id).await?.try_into()?))
}
