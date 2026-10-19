//! 仲裁的共享状态：论坛裁决 Passkey 的 Webauthn 实例（RP ID 为论坛主域名，与钱包 Passkey 是两组凭证）、
//! 裁决页面来源校验、会话里的「近期验证」标记、仲裁 Safe 地址。

use axum::extract::{FromRef, FromRequestParts};
use axum::http::header;
use axum::http::request::Parts;
use forum_auth::passkey::UserHandles;
use forum_auth::passkey::webauthn::Webauthn;
use forum_core::ErrorCode;
use time::OffsetDateTime;
use tower_sessions::Session;

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// 邮箱验证有效 10 分钟（与验证码有效期一致）：只用于登记第一个裁决 Passkey。
pub const EMAIL_REVERIFY_SECS: i64 = 600;
/// 裁决 Passkey 近期验证有效 5 分钟：再登记一个 Passkey（第二台设备）时要求。
pub const PASSKEY_RECENT_SECS: i64 = 300;
/// 每人最多登记的裁决 Passkey 数。
pub const MAX_PASSKEYS: usize = 5;

const SESSION_EMAIL_REVERIFIED: &str = "ruling_email_at";
const SESSION_PASSKEY_VERIFIED: &str = "ruling_passkey_at";

pub struct ArbitrationState {
    pub webauthn: Webauthn,
    /// 论坛页面的 Origin（与请求头逐字比较）。
    pub origin: String,
    pub handles: UserHandles,
    pub brand: String,
    /// 仲裁 Safe；只写进给签名人导入的 Safe 交易文件。
    pub safe: Option<alloy::primitives::Address>,
}

/// 只接受来自论坛页面（`RULING_ORIGIN`）的请求：裁决 Passkey 的 challenge 与登记不能由钱包或其他来源发起。
#[derive(Debug, Clone, Copy)]
pub struct RulingOrigin;

impl<S> FromRequestParts<S> for RulingOrigin
where
    S: Send + Sync,
    AppState: FromRef<S>,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = AppState::from_ref(state);
        let ok = parts
            .headers
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|o| o == state.arbitration.origin);
        if ok {
            Ok(Self)
        } else {
            Err(ApiError(ErrorCode::OriginRejected))
        }
    }
}

fn now() -> i64 {
    OffsetDateTime::now_utc().unix_timestamp()
}

fn within(at: Option<i64>, secs: i64) -> bool {
    at.is_some_and(|t| now() - t <= secs)
}

pub async fn mark_email_reverified(session: &Session) -> ApiResult<()> {
    session.insert(SESSION_EMAIL_REVERIFIED, now()).await?;
    Ok(())
}

/// 邮箱验证标记：`consume` 为真时取出即作废（只能用一次）。
pub async fn email_reverified(session: &Session, consume: bool) -> ApiResult<bool> {
    let at = if consume {
        session.remove::<i64>(SESSION_EMAIL_REVERIFIED).await?
    } else {
        session.get::<i64>(SESSION_EMAIL_REVERIFIED).await?
    };
    Ok(within(at, EMAIL_REVERIFY_SECS))
}

pub async fn mark_passkey_verified(session: &Session) -> ApiResult<i64> {
    let at = now();
    session.insert(SESSION_PASSKEY_VERIFIED, at).await?;
    Ok(at + PASSKEY_RECENT_SECS)
}

pub async fn passkey_recent(session: &Session) -> ApiResult<bool> {
    let at = session.get::<i64>(SESSION_PASSKEY_VERIFIED).await?;
    Ok(within(at, PASSKEY_RECENT_SECS))
}
