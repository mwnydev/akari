//! 邮箱验证码登录（plan 第 5 节）。普通用户只有这一种登录方式，不设密码、不设登录 Passkey。

use std::net::IpAddr;
use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum_extra::extract::SignedCookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use forum_auth::{Captcha, CodeError};
use forum_core::staff::{Role, StaffRoles};
use forum_core::{ErrorCode, normalize_email};
use forum_db::outbox;
use forum_kv::SlidingWindow;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use time::format_description::well_known::Rfc3339;
use tower_sessions::Session;
use utoipa::ToSchema;

use super::SESSION_USER_ID;
use super::me::Me;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::middleware::ClientIp;
use crate::state::AppState;

/// 每个 IP 每小时最多请求 20 次验证码；每个邮箱每小时 5 次（另有 60 秒重发间隔）。
const START_PER_IP: SlidingWindow =
    SlidingWindow::new("auth-start-ip", 20, Duration::from_secs(3600));
const START_PER_EMAIL: SlidingWindow =
    SlidingWindow::new("auth-start-email", 5, Duration::from_secs(3600));
/// 校验接口按 IP 限流，防止换邮箱撞码。
const VERIFY_PER_IP: SlidingWindow =
    SlidingWindow::new("auth-verify-ip", 30, Duration::from_secs(600));

/// 设备 Cookie：长期有效、签名，用于识别「新设备登录」。
pub const DEVICE_COOKIE: &str = "did";
const DEVICE_COOKIE_DAYS: i64 = 400;

pub const KIND_LOGIN_CODE: &str = "email.login_code";
pub const KIND_NEW_DEVICE: &str = "email.new_device";

#[derive(Debug, Deserialize, ToSchema)]
pub struct StartRequest {
    pub email: String,
    /// Cloudflare Turnstile 令牌。
    pub captcha: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct StartResponse {
    /// 验证码有效秒数。
    pub expires_in: u64,
    /// 多少秒后可重发。
    pub resend_in: u64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct VerifyRequest {
    pub email: String,
    pub code: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct VerifyResponse {
    pub user: Me,
    /// 本次登录是否新注册。
    pub created: bool,
}

async fn limit(state: &AppState, rule: &SlidingWindow, id: &str) -> ApiResult<()> {
    if rule.hit(&state.kv, id).await? {
        Ok(())
    } else {
        Err(ApiError(ErrorCode::RateLimited))
    }
}

/// 发送登录验证码。不论邮箱是否已注册，返回都一样。
#[utoipa::path(
    post, path = "/api/v1/auth/email/start", tag = "auth",
    request_body = StartRequest,
    responses(
        (status = 202, body = StartResponse),
        (status = 400, body = ErrorBody), (status = 403, body = ErrorBody), (status = 429, body = ErrorBody)
    )
)]
pub async fn start(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    jar: SignedCookieJar,
    Json(req): Json<StartRequest>,
) -> ApiResult<(StatusCode, Json<StartResponse>)> {
    let email = normalize_email(&req.email)?;
    limit(&state, &START_PER_IP, &ip.to_string()).await?;
    limit(&state, &START_PER_EMAIL, &state.codes.email_tag(&email)).await?;
    check_access(&state, ip, device_id(&jar).as_deref()).await?;
    let passed = state
        .captcha
        .verify(req.captcha.as_deref(), ip)
        .await
        .map_err(ApiError::internal)?;
    if !passed {
        return Err(ApiError(ErrorCode::CaptchaFailed));
    }
    let code = state.codes.issue(&email).await.map_err(code_error)?;
    outbox::enqueue(
        &state.db,
        KIND_LOGIN_CODE,
        &json!({ "to": email, "code": code }),
        outbox::PRIORITY_HIGH,
    )
    .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(StartResponse {
            expires_in: forum_auth::code::CODE_TTL.as_secs(),
            resend_in: forum_auth::code::RESEND_INTERVAL.as_secs(),
        }),
    ))
}

/// 设备 Cookie 里的设备 ID（签名有效且格式正确才算）。
fn device_id(jar: &SignedCookieJar) -> Option<String> {
    jar.get(DEVICE_COOKIE)
        .map(|c| c.value().to_owned())
        .filter(|v| v.len() == 32 && v.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// IP / 设备封禁（ADR 0013）：被封的网络或设备不能登录、不能注册。
async fn check_access(state: &AppState, ip: IpAddr, device_id: Option<&str>) -> ApiResult<()> {
    let hash = device_id.map(|d| Sha256::digest(d.as_bytes()));
    let banned = forum_db::sanctions::access_banned(
        &state.db,
        &ip.to_string(),
        hash.as_ref().map(|h| h.as_slice()),
    )
    .await?;
    if banned {
        Err(ApiError(ErrorCode::AccessBanned))
    } else {
        Ok(())
    }
}

fn role_names(roles: &StaffRoles) -> String {
    roles
        .roles()
        .into_iter()
        .map(Role::label)
        .collect::<Vec<_>>()
        .join("、")
}

pub(crate) fn code_error(e: CodeError) -> ApiError {
    match e {
        CodeError::ResendTooSoon => ApiError(ErrorCode::ResendTooSoon),
        CodeError::Invalid => ApiError(ErrorCode::InvalidCode),
        CodeError::TooManyAttempts => ApiError(ErrorCode::TooManyAttempts),
        CodeError::Kv(e) => ApiError::internal(e),
    }
}

/// 校验验证码并登录；新邮箱自动注册。
#[utoipa::path(
    post, path = "/api/v1/auth/email/verify", tag = "auth",
    request_body = VerifyRequest,
    responses(
        (status = 200, body = VerifyResponse),
        (status = 400, body = ErrorBody), (status = 403, body = ErrorBody), (status = 429, body = ErrorBody)
    )
)]
pub async fn verify(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    session: Session,
    jar: SignedCookieJar,
    Json(req): Json<VerifyRequest>,
) -> ApiResult<(SignedCookieJar, Json<VerifyResponse>)> {
    let email = normalize_email(&req.email)?;
    limit(&state, &VERIFY_PER_IP, &ip.to_string()).await?;
    state
        .codes
        .verify(&email, &req.code)
        .await
        .map_err(code_error)?;

    let known_device = device_id(&jar);
    check_access(&state, ip, known_device.as_deref()).await?;
    let device_id = known_device.unwrap_or_else(|| hex::encode(rand::rng().random::<[u8; 16]>()));
    let device_hash = Sha256::digest(device_id.as_bytes());

    // 用户、设备与「新设备提醒」同一事务写入（事务发件箱）。
    let mut tx = state.db.begin().await?;
    let (user, created) = forum_db::users::upsert_by_email(&mut *tx, &email).await?;
    let new_device = forum_db::users::touch_device(&mut *tx, user.id, &device_hash).await?;
    let roles = forum_db::staff::roles(&mut *tx, user.id).await?;
    if new_device && !created {
        let at = time::OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .map_err(ApiError::internal)?;
        outbox::enqueue(
            &mut *tx,
            KIND_NEW_DEVICE,
            &json!({ "to": user.email, "at": at }),
            outbox::PRIORITY_NORMAL,
        )
        .await?;
        // 管理账号只靠邮箱保护：新设备登录另推管理员群（ADR 0013 第 6 条）。
        if roles.is_staff() {
            let name = user.username.as_deref().unwrap_or("（未设置用户名）");
            let text = format!(
                "管理人员新设备登录：#{} {name}（{}），IP {ip}，时间 {at}。如非本人操作，请站长立即撤销其角色。",
                user.id,
                role_names(&roles)
            );
            outbox::staff_alert(&mut *tx, "staff.new_device", &text).await?;
        }
    }
    tx.commit().await?;

    // 登录后轮换会话 ID，防会话固定攻击。
    session.cycle_id().await?;
    session.insert(SESSION_USER_ID, user.id).await?;

    let cookie = Cookie::build((DEVICE_COOKIE, device_id))
        .path("/")
        .http_only(true)
        .secure(state.cookie_secure)
        .same_site(SameSite::Lax)
        .max_age(time::Duration::days(DEVICE_COOKIE_DAYS))
        .build();
    Ok((
        jar.add(cookie),
        Json(VerifyResponse {
            user: Me::new(user, &roles),
            created,
        }),
    ))
}

/// 退出登录：删除服务端会话。
#[utoipa::path(post, path = "/api/v1/auth/logout", tag = "auth", responses((status = 204)))]
pub async fn logout(session: Session) -> ApiResult<StatusCode> {
    session.flush().await?;
    Ok(StatusCode::NO_CONTENT)
}
