//! 统一错误响应 `{code, message}`；内部错误只记日志，不向外泄露细节。

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use forum_core::ErrorCode;
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: &'static str,
}

#[derive(Debug)]
pub struct ApiError(pub ErrorCode);

impl ApiError {
    /// 记录内部错误并返回 `INTERNAL`。
    pub fn internal(err: impl std::fmt::Display) -> Self {
        tracing::error!(error = %err, "internal error");
        Self(ErrorCode::Internal)
    }
}

impl From<ErrorCode> for ApiError {
    fn from(code: ErrorCode) -> Self {
        Self(code)
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        Self::internal(e)
    }
}

impl From<forum_content::ContentError> for ApiError {
    fn from(e: forum_content::ContentError) -> Self {
        Self(match e {
            forum_content::ContentError::Empty => ErrorCode::ContentEmpty,
            forum_content::ContentError::TooLong => ErrorCode::ContentTooLong,
            forum_content::ContentError::InvalidTitle => ErrorCode::InvalidTitle,
        })
    }
}

impl From<forum_kv::KvError> for ApiError {
    fn from(e: forum_kv::KvError) -> Self {
        Self::internal(e)
    }
}

impl From<tower_sessions::session::Error> for ApiError {
    fn from(e: tower_sessions::session::Error) -> Self {
        Self::internal(e)
    }
}

fn status(code: ErrorCode) -> StatusCode {
    match code {
        ErrorCode::Unauthorized => StatusCode::UNAUTHORIZED,
        ErrorCode::OriginRejected
        | ErrorCode::CaptchaFailed
        | ErrorCode::Forbidden
        | ErrorCode::TopicLocked
        | ErrorCode::ExternalLinkForbidden
        | ErrorCode::ReverifyRequired
        | ErrorCode::PasskeyRequired
        | ErrorCode::WalletRequired => StatusCode::FORBIDDEN,
        ErrorCode::OfferUnavailable | ErrorCode::Conflict => StatusCode::CONFLICT,
        ErrorCode::NotFound => StatusCode::NOT_FOUND,
        ErrorCode::ResendTooSoon | ErrorCode::RateLimited | ErrorCode::TooManyAttempts => {
            StatusCode::TOO_MANY_REQUESTS
        }
        ErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        ErrorCode::AmountOverflow
        | ErrorCode::AmountUnderflow
        | ErrorCode::InvalidParameter
        | ErrorCode::ClaimBelowGas
        | ErrorCode::InvalidEmail
        | ErrorCode::InvalidCode
        | ErrorCode::ContentEmpty
        | ErrorCode::ContentTooLong
        | ErrorCode::InvalidTitle
        | ErrorCode::AmountOutOfRange
        | ErrorCode::WindowOutOfRange
        | ErrorCode::BondInvalid
        | ErrorCode::FeeLimitInvalid
        | ErrorCode::OfferExpiryInvalid
        | ErrorCode::UsernameRequired
        | ErrorCode::PasskeyFailed
        | ErrorCode::TooManyPasskeys
        | ErrorCode::WalletAddressMismatch => StatusCode::BAD_REQUEST,
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorBody {
            code: self.0,
            message: self.0.message(),
        };
        (status(self.0), Json(body)).into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
