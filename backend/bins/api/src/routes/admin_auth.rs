//! 站长后台登录（ADR 0013 第 4 条）：后台 Passkey 绑定、登录、敏感操作前再验证、退出。会话规则见 [`crate::admin`]。
//!
//! - 绑定**第一个**后台 Passkey：已用邮箱登录论坛的站长账号 + 10 分钟内专门为此做过的邮箱验证码（一次性，
//!   与「开通交易」同样的流程，邮件写明用途）。已有后台 Passkey 时不能再走邮箱，只能在后台会话里再验证后添加，
//!   单凭邮箱拿不到后台。
//! - 登录：论坛会话里的账号必须是站长，再用该账号的后台 Passkey 验证；之后后台会话独立于论坛会话。
//! - 每次绑定都推送管理员群。

use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum_extra::extract::CookieJar;
use forum_auth::passkey::webauthn::{
    CreationChallengeResponse, CredentialID, Passkey, PasskeyAuthentication, PasskeyRegistration,
    PublicKeyCredential, RegisterPublicKeyCredential, RequestChallengeResponse,
};
use forum_core::ErrorCode;
use forum_db::outbox;
use forum_db::staff::{self, AdminPasskey};
use forum_kv::SlidingWindow;
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::OffsetDateTime;
use tower_sessions::Session;
use utoipa::ToSchema;

use super::auth::code_error;
use super::wallet::{base64url, current_user, save_challenge, take_challenge};
use crate::admin::AdminSession;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::posting::limit;
use crate::state::AppState;

pub const KIND_ADMIN_CODE: &str = "email.admin_code";
/// 每个站长最多登记的后台 Passkey 数。
pub const MAX_ADMIN_PASSKEYS: usize = 5;
/// 绑定第一个 Passkey 的邮箱验证有效 10 分钟、只能用一次（论坛会话里的标记）。
const ENROLL_SECS: i64 = 600;
const SESSION_ENROLL_AT: &str = "admin_enroll_at";

const CODE_RATE: SlidingWindow = SlidingWindow::new("admin-code", 5, Duration::from_secs(3600));
const CEREMONY_RATE: SlidingWindow =
    SlidingWindow::new("admin-ceremony", 30, Duration::from_secs(600));

/// 后台验证码与登录、开通交易验证码分开存放，不能互用。
fn enroll_subject(email: &str) -> String {
    format!("admin-enroll\0{email}")
}

fn now() -> i64 {
    OffsetDateTime::now_utc().unix_timestamp()
}

/// 论坛会话里的站长账号（绑定第一个 Passkey 与登录用）。
async fn forum_owner(state: &AppState, session: &Session) -> ApiResult<forum_db::users::User> {
    let user = current_user(state, session).await?;
    if !staff::roles(&state.db, user.id).await?.owner {
        return Err(ApiError(ErrorCode::Forbidden));
    }
    Ok(user)
}

fn parse(p: &AdminPasskey) -> ApiResult<Passkey> {
    serde_json::from_value(p.credential.clone()).map_err(ApiError::internal)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AdminCodeSent {
    pub expires_in: u64,
    pub resend_in: u64,
}

/// 绑定第一个后台 Passkey 前：向站长账号邮箱发一封专用验证码。已有后台 Passkey 时返回 `CONFLICT`。
#[utoipa::path(
    post, path = "/api/v1/admin/enroll/code", tag = "admin",
    responses(
        (status = 202, body = AdminCodeSent),
        (status = 401, body = ErrorBody), (status = 403, body = ErrorBody), (status = 409, body = ErrorBody),
        (status = 429, body = ErrorBody)
    )
)]
pub async fn admin_enroll_code(
    State(state): State<AppState>,
    session: Session,
) -> ApiResult<(StatusCode, Json<AdminCodeSent>)> {
    let user = forum_owner(&state, &session).await?;
    if !staff::admin_passkeys(&state.db, user.id).await?.is_empty() {
        return Err(ApiError(ErrorCode::Conflict));
    }
    limit(&state, &CODE_RATE, user.id).await?;
    let code = state
        .codes
        .issue(&enroll_subject(&user.email))
        .await
        .map_err(code_error)?;
    outbox::enqueue(
        &state.db,
        KIND_ADMIN_CODE,
        &json!({ "to": user.email, "code": code }),
        outbox::PRIORITY_HIGH,
    )
    .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(AdminCodeSent {
            expires_in: forum_auth::code::CODE_TTL.as_secs(),
            resend_in: forum_auth::code::RESEND_INTERVAL.as_secs(),
        }),
    ))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AdminCodeRequest {
    pub code: String,
}

/// 校验验证码；成功后 10 分钟内可绑定第一个后台 Passkey。
#[utoipa::path(
    post, path = "/api/v1/admin/enroll/verify", tag = "admin",
    request_body = AdminCodeRequest,
    responses(
        (status = 204),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 429, body = ErrorBody)
    )
)]
pub async fn admin_enroll_verify(
    State(state): State<AppState>,
    session: Session,
    Json(req): Json<AdminCodeRequest>,
) -> ApiResult<StatusCode> {
    let user = forum_owner(&state, &session).await?;
    state
        .codes
        .verify(&enroll_subject(&user.email), &req.code)
        .await
        .map_err(code_error)?;
    session.insert(SESSION_ENROLL_AT, now()).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// 绑定方式：第一个（论坛会话 + 邮箱验证）或再加一个（后台会话 + 5 分钟内再验证）。
enum Enrollment {
    First(forum_db::users::User),
    Another(AdminSession),
}

impl Enrollment {
    async fn resolve(state: &AppState, session: &Session, jar: &CookieJar) -> ApiResult<Self> {
        if let Some(admin) = AdminSession::load(state, jar).await? {
            admin.require_recent()?;
            return Ok(Self::Another(admin));
        }
        let user = forum_owner(state, session).await?;
        if !staff::admin_passkeys(&state.db, user.id).await?.is_empty() {
            // 已有后台 Passkey：必须先登录后台
            return Err(ApiError(ErrorCode::Unauthorized));
        }
        Ok(Self::First(user))
    }

    fn user_id(&self) -> i64 {
        match self {
            Self::First(u) => u.id,
            Self::Another(a) => a.user_id,
        }
    }
}

fn enroll_marked(at: Option<i64>) -> bool {
    at.is_some_and(|t| now() - t <= ENROLL_SECS)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AdminRegistrationOptions {
    /// 原样传给 `navigator.credentials.create()`。
    #[schema(value_type = Object)]
    pub options: CreationChallengeResponse,
}

/// 开始绑定后台 Passkey（RP ID 为后台中心域名）。
#[utoipa::path(
    post, path = "/api/v1/admin/passkeys/register/start", tag = "admin",
    responses(
        (status = 200, body = AdminRegistrationOptions),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 429, body = ErrorBody)
    )
)]
pub async fn admin_register_start(
    State(state): State<AppState>,
    session: Session,
    jar: CookieJar,
) -> ApiResult<Json<AdminRegistrationOptions>> {
    let enrollment = Enrollment::resolve(&state, &session, &jar).await?;
    let user_id = enrollment.user_id();
    if matches!(enrollment, Enrollment::First(_))
        && !enroll_marked(session.get::<i64>(SESSION_ENROLL_AT).await?)
    {
        return Err(ApiError(ErrorCode::ReverifyRequired));
    }
    let existing = staff::admin_passkeys(&state.db, user_id).await?;
    if existing.len() >= MAX_ADMIN_PASSKEYS {
        return Err(ApiError(ErrorCode::TooManyPasskeys));
    }
    limit(&state, &CEREMONY_RATE, user_id).await?;
    let a = &state.admin;
    let name = format!("{} 站长后台（勿删）· #{user_id}", state.wallet.brand);
    let exclude: Vec<CredentialID> = existing
        .iter()
        .map(|p| p.credential_id.clone().into())
        .collect();
    let (options, reg) = a
        .webauthn
        .start_passkey_registration(
            a.handles.of(user_id),
            &name,
            &name,
            (!exclude.is_empty()).then_some(exclude),
        )
        .map_err(ApiError::internal)?;
    save_challenge(&state, "admin:reg", user_id, &reg).await?;
    Ok(Json(AdminRegistrationOptions { options }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AdminRegisterFinish {
    #[schema(value_type = Object)]
    pub credential: RegisterPublicKeyCredential,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AdminPasskeyView {
    pub id: i64,
    /// base64url 凭证 ID。
    pub credential_id: String,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[schema(value_type = Option<String>, format = DateTime)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_used_at: Option<OffsetDateTime>,
}

impl From<&AdminPasskey> for AdminPasskeyView {
    fn from(p: &AdminPasskey) -> Self {
        Self {
            id: p.id,
            credential_id: base64url(&p.credential_id),
            created_at: p.created_at,
            last_used_at: p.last_used_at,
        }
    }
}

/// 完成绑定。绑定第一个时消耗邮箱验证标记并直接登录后台。
#[utoipa::path(
    post, path = "/api/v1/admin/passkeys/register/finish", tag = "admin",
    request_body = AdminRegisterFinish,
    responses(
        (status = 201, body = AdminPasskeyView),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 409, body = ErrorBody)
    )
)]
pub async fn admin_register_finish(
    State(state): State<AppState>,
    session: Session,
    jar: CookieJar,
    Json(req): Json<AdminRegisterFinish>,
) -> ApiResult<(StatusCode, CookieJar, Json<AdminPasskeyView>)> {
    let enrollment = Enrollment::resolve(&state, &session, &jar).await?;
    let user_id = enrollment.user_id();
    let reg: PasskeyRegistration = take_challenge(&state, "admin:reg", user_id).await?;
    let passkey = state
        .admin
        .webauthn
        .finish_passkey_registration(&req.credential, &reg)
        .map_err(|e| {
            tracing::info!(error = %e, "admin passkey registration rejected");
            ApiError(ErrorCode::PasskeyFailed)
        })?;
    // 开始绑定后才检查的条件在这里复核
    if matches!(enrollment, Enrollment::First(_))
        && !enroll_marked(session.remove::<i64>(SESSION_ENROLL_AT).await?)
    {
        return Err(ApiError(ErrorCode::ReverifyRequired));
    }
    if staff::admin_passkeys(&state.db, user_id).await?.len() >= MAX_ADMIN_PASSKEYS {
        return Err(ApiError(ErrorCode::TooManyPasskeys));
    }
    let credential = serde_json::to_value(&passkey).map_err(ApiError::internal)?;
    let mut tx = state.db.begin().await?;
    let id =
        staff::insert_admin_passkey(&mut *tx, user_id, passkey.cred_id().as_ref(), &credential)
            .await?
            .ok_or(ApiError(ErrorCode::Conflict))?;
    forum_db::audit::append(
        &mut *tx,
        user_id,
        "admin.passkey_add",
        Some(&format!("passkey:{id}")),
        None,
        &json!({ "first": matches!(enrollment, Enrollment::First(_)) }),
    )
    .await?;
    outbox::staff_alert(
        &mut *tx,
        "admin.passkey_add",
        &format!("站长账号 #{user_id} 绑定了新的后台 Passkey（#{id}）。如非本人操作，请立即处理。"),
    )
    .await?;
    tx.commit().await?;
    let jar = match enrollment {
        Enrollment::First(_) => AdminSession::create(&state, jar, user_id).await?.0,
        Enrollment::Another(_) => jar,
    };
    let saved = staff::admin_passkeys(&state.db, user_id)
        .await?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| ApiError::internal("admin passkey vanished after insert"))?;
    Ok((
        StatusCode::CREATED,
        jar,
        Json(AdminPasskeyView::from(&saved)),
    ))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AdminPasskeyList {
    pub passkeys: Vec<AdminPasskeyView>,
}

/// 我的后台 Passkey。
#[utoipa::path(
    get, path = "/api/v1/admin/passkeys", tag = "admin",
    responses((status = 200, body = AdminPasskeyList), (status = 401, body = ErrorBody))
)]
pub async fn admin_passkeys(
    State(state): State<AppState>,
    admin: AdminSession,
) -> ApiResult<Json<AdminPasskeyList>> {
    let rows = staff::admin_passkeys(&state.db, admin.user_id).await?;
    Ok(Json(AdminPasskeyList {
        passkeys: rows.iter().map(AdminPasskeyView::from).collect(),
    }))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AdminAssertionOptions {
    /// 原样传给 `navigator.credentials.get()`。
    #[schema(value_type = Object)]
    pub options: RequestChallengeResponse,
}

async fn assertion_start(
    state: &AppState,
    kind: &str,
    user_id: i64,
) -> ApiResult<Json<AdminAssertionOptions>> {
    let stored = staff::admin_passkeys(&state.db, user_id).await?;
    if stored.is_empty() {
        return Err(ApiError(ErrorCode::ReverifyRequired));
    }
    limit(state, &CEREMONY_RATE, user_id).await?;
    let creds = stored.iter().map(parse).collect::<ApiResult<Vec<_>>>()?;
    let (options, auth) = state
        .admin
        .webauthn
        .start_passkey_authentication(&creds)
        .map_err(ApiError::internal)?;
    save_challenge(state, kind, user_id, &auth).await?;
    Ok(Json(AdminAssertionOptions { options }))
}

/// 校验断言并写回签名计数。
async fn assertion_finish(
    state: &AppState,
    kind: &str,
    user_id: i64,
    credential: &PublicKeyCredential,
) -> ApiResult<()> {
    let auth: PasskeyAuthentication = take_challenge(state, kind, user_id).await?;
    let result = state
        .admin
        .webauthn
        .finish_passkey_authentication(credential, &auth)
        .map_err(|e| {
            tracing::info!(error = %e, "admin passkey assertion rejected");
            ApiError(ErrorCode::PasskeyFailed)
        })?;
    let stored = staff::admin_passkeys(&state.db, user_id).await?;
    let row = stored
        .iter()
        .find(|p| p.credential_id.as_slice() == result.cred_id().as_ref())
        .ok_or(ApiError(ErrorCode::PasskeyFailed))?;
    let mut passkey = parse(row)?;
    passkey.update_credential(&result);
    let updated = serde_json::to_value(&passkey).map_err(ApiError::internal)?;
    forum_db::wallet::touch_passkey(&state.db, row.id, &updated).await?;
    Ok(())
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AdminAssertFinish {
    #[schema(value_type = Object)]
    pub credential: PublicKeyCredential,
}

/// 开始登录后台：论坛会话里的账号必须是站长且已绑定后台 Passkey（未绑定返回 `REVERIFY_REQUIRED`，先走邮箱验证绑定）。
#[utoipa::path(
    post, path = "/api/v1/admin/login/start", tag = "admin",
    responses(
        (status = 200, body = AdminAssertionOptions),
        (status = 401, body = ErrorBody), (status = 403, body = ErrorBody), (status = 429, body = ErrorBody)
    )
)]
pub async fn admin_login_start(
    State(state): State<AppState>,
    session: Session,
) -> ApiResult<Json<AdminAssertionOptions>> {
    let user = forum_owner(&state, &session).await?;
    assertion_start(&state, "admin:login", user.id).await
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AdminSessionView {
    pub user_id: i64,
    /// 会话到期时间（Unix 秒，闲置与最长时长取先到者）。
    pub expires_at: i64,
    /// 最长时长到期（Unix 秒）。
    pub absolute_expires_at: i64,
    /// 敏感操作再验证有效至（Unix 秒）；为空表示需要再验证。
    pub reverified_until: Option<i64>,
}

impl From<&AdminSession> for AdminSessionView {
    fn from(s: &AdminSession) -> Self {
        Self {
            user_id: s.user_id,
            expires_at: s.expires_at(),
            absolute_expires_at: s.absolute_expires_at(),
            reverified_until: s.reverified_until(),
        }
    }
}

/// 完成登录：发放后台会话 Cookie。
#[utoipa::path(
    post, path = "/api/v1/admin/login/finish", tag = "admin",
    request_body = AdminAssertFinish,
    responses(
        (status = 200, body = AdminSessionView),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody)
    )
)]
pub async fn admin_login_finish(
    State(state): State<AppState>,
    session: Session,
    jar: CookieJar,
    Json(req): Json<AdminAssertFinish>,
) -> ApiResult<(CookieJar, Json<AdminSessionView>)> {
    let user = forum_owner(&state, &session).await?;
    assertion_finish(&state, "admin:login", user.id, &req.credential).await?;
    let (jar, admin) = AdminSession::create(&state, jar, user.id).await?;
    Ok((jar, Json(AdminSessionView::from(&admin))))
}

/// 当前后台会话。
#[utoipa::path(
    get, path = "/api/v1/admin/session", tag = "admin",
    responses((status = 200, body = AdminSessionView), (status = 401, body = ErrorBody))
)]
pub async fn admin_current(admin: AdminSession) -> Json<AdminSessionView> {
    Json(AdminSessionView::from(&admin))
}

/// 开始再验证（敏感操作前）。
#[utoipa::path(
    post, path = "/api/v1/admin/reverify/start", tag = "admin",
    responses(
        (status = 200, body = AdminAssertionOptions),
        (status = 401, body = ErrorBody), (status = 429, body = ErrorBody)
    )
)]
pub async fn admin_reverify_start(
    State(state): State<AppState>,
    admin: AdminSession,
) -> ApiResult<Json<AdminAssertionOptions>> {
    assertion_start(&state, "admin:reverify", admin.user_id).await
}

/// 完成再验证：5 分钟内可做敏感操作。
#[utoipa::path(
    post, path = "/api/v1/admin/reverify/finish", tag = "admin",
    request_body = AdminAssertFinish,
    responses(
        (status = 200, body = AdminSessionView),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody)
    )
)]
pub async fn admin_reverify_finish(
    State(state): State<AppState>,
    mut admin: AdminSession,
    Json(req): Json<AdminAssertFinish>,
) -> ApiResult<Json<AdminSessionView>> {
    assertion_finish(&state, "admin:reverify", admin.user_id, &req.credential).await?;
    admin.mark_reverified(&state).await?;
    Ok(Json(AdminSessionView::from(&admin)))
}

/// 退出后台。
#[utoipa::path(
    post, path = "/api/v1/admin/logout", tag = "admin",
    responses((status = 204), (status = 401, body = ErrorBody))
)]
pub async fn admin_logout(
    State(state): State<AppState>,
    admin: AdminSession,
    jar: CookieJar,
) -> ApiResult<(StatusCode, CookieJar)> {
    Ok((StatusCode::NO_CONTENT, admin.destroy(&state, jar).await?))
}
