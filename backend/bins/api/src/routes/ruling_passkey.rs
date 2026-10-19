//! 论坛裁决 Passkey（ADR 0012、0013 第 4 条）：站长与管理员提交裁决时用它确认，因为裁决决定托管资金的分配。
//!
//! 安全决定：
//! - RP ID 为论坛主域名（生产 `akari.cc`），只接受论坛页面来源（[`RulingOrigin`]）；与钱包 Passkey（RP `wallet.akari.cc`）
//!   是两组凭证：Webauthn 实例、用户句柄、数据库 `purpose` 都分开，钱包接口查不到裁决凭证，反之亦然。
//! - 只有能接案的人（站长、管理员）能登记。第一个需要 10 分钟内专门为此做过的邮箱验证（一次性，登记成功即消耗）；
//!   再登记一个需要 5 分钟内用已有的裁决 Passkey 验证过。
//! - challenge 存 Valkey 5 分钟，取出即删除，不能重放。签署裁决的断言见 `routes::arbitration`（challenge 绑定裁决内容）。

use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use forum_auth::passkey::webauthn::{
    CreationChallengeResponse, Passkey, PasskeyAuthentication, PasskeyRegistration,
    PublicKeyCredential, RegisterPublicKeyCredential, RequestChallengeResponse,
};
use forum_core::ErrorCode;
use forum_db::arbitration::{self as db, RulingPasskey};
use forum_db::outbox;
use forum_kv::SlidingWindow;
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::OffsetDateTime;
use tower_sessions::Session;
use utoipa::ToSchema;

use super::auth::code_error;
use super::me::current_user_id;
use super::wallet::{ReverifyRequest, ReverifyStarted, current_user};
use crate::arbitration::{
    MAX_PASSKEYS, RulingOrigin, email_reverified, mark_email_reverified, mark_passkey_verified,
    passkey_recent, save_challenge, take_challenge,
};
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::posting::limit;
use crate::state::AppState;

pub const KIND_RULING_CODE: &str = "email.ruling_passkey_code";

const REVERIFY_RATE: SlidingWindow =
    SlidingWindow::new("ruling-reverify", 5, Duration::from_secs(3600));
pub(crate) const CEREMONY_RATE: SlidingWindow =
    SlidingWindow::new("ruling-ceremony", 30, Duration::from_secs(600));

/// 与登录码、开通交易码分开存放：别的验证码不能拿来登记裁决 Passkey。
fn reverify_subject(email: &str) -> String {
    format!("ruling-reverify\0{email}")
}

/// 只有能接案的人（站长、管理员）能用这些接口；其他人一律 403。
pub(crate) async fn require_eligible(state: &AppState, user_id: i64) -> ApiResult<()> {
    if db::is_eligible(&state.db, user_id).await? {
        Ok(())
    } else {
        Err(ApiError(ErrorCode::Forbidden))
    }
}

pub(crate) fn parse_passkey(p: &RulingPasskey) -> ApiResult<Passkey> {
    serde_json::from_value(p.credential.clone()).map_err(ApiError::internal)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RulingPasskeyView {
    pub id: i64,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[schema(value_type = Option<String>, format = DateTime)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_used_at: Option<OffsetDateTime>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RulingPasskeys {
    pub passkeys: Vec<RulingPasskeyView>,
}

/// 我的裁决 Passkey。
#[utoipa::path(
    get, path = "/api/v1/arbitration/passkeys", tag = "arbitration",
    responses((status = 200, body = RulingPasskeys), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody))
)]
pub async fn list_ruling_passkeys(
    State(state): State<AppState>,
    session: Session,
) -> ApiResult<Json<RulingPasskeys>> {
    let me = current_user_id(&session).await?;
    require_eligible(&state, me).await?;
    let passkeys = db::ruling_passkeys(&state.db, me)
        .await?
        .into_iter()
        .map(|p| RulingPasskeyView {
            id: p.id,
            created_at: p.created_at,
            last_used_at: p.last_used_at,
        })
        .collect();
    Ok(Json(RulingPasskeys { passkeys }))
}

/// 登记第一个裁决 Passkey 前的邮箱验证：向账号邮箱发一封写明「登记裁决 Passkey」的验证码。
#[utoipa::path(
    post, path = "/api/v1/arbitration/passkey/reverify/start", tag = "arbitration",
    responses(
        (status = 202, body = ReverifyStarted),
        (status = 401, body = ErrorBody), (status = 403, body = ErrorBody), (status = 429, body = ErrorBody)
    )
)]
pub async fn ruling_reverify_start(
    State(state): State<AppState>,
    _: RulingOrigin,
    session: Session,
) -> ApiResult<(StatusCode, Json<ReverifyStarted>)> {
    let user = current_user(&state, &session).await?;
    require_eligible(&state, user.id).await?;
    limit(&state, &REVERIFY_RATE, user.id).await?;
    let code = state
        .codes
        .issue(&reverify_subject(&user.email))
        .await
        .map_err(code_error)?;
    outbox::enqueue(
        &state.db,
        KIND_RULING_CODE,
        &json!({ "to": user.email, "code": code }),
        outbox::PRIORITY_HIGH,
    )
    .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(ReverifyStarted {
            expires_in: forum_auth::code::CODE_TTL.as_secs(),
            resend_in: forum_auth::code::RESEND_INTERVAL.as_secs(),
        }),
    ))
}

/// 校验邮箱验证码；成功后 10 分钟内可登记第一个裁决 Passkey。
#[utoipa::path(
    post, path = "/api/v1/arbitration/passkey/reverify/verify", tag = "arbitration",
    request_body = ReverifyRequest,
    responses(
        (status = 204),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 429, body = ErrorBody)
    )
)]
pub async fn ruling_reverify_verify(
    State(state): State<AppState>,
    _: RulingOrigin,
    session: Session,
    Json(req): Json<ReverifyRequest>,
) -> ApiResult<StatusCode> {
    let user = current_user(&state, &session).await?;
    require_eligible(&state, user.id).await?;
    state
        .codes
        .verify(&reverify_subject(&user.email), &req.code)
        .await
        .map_err(code_error)?;
    mark_email_reverified(&session).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// 登记的前提：第一个要邮箱验证（`consume` 时消耗），之后的要近期用已有裁决 Passkey 验证过。
async fn enrollment_allowed(
    session: &Session,
    existing: &[RulingPasskey],
    consume: bool,
) -> ApiResult<()> {
    if existing.len() >= MAX_PASSKEYS {
        return Err(ApiError(ErrorCode::TooManyPasskeys));
    }
    let ok = if existing.is_empty() {
        email_reverified(session, consume).await?
    } else {
        passkey_recent(session).await?
    };
    match (ok, existing.is_empty()) {
        (true, _) => Ok(()),
        (false, true) => Err(ApiError(ErrorCode::ReverifyRequired)),
        (false, false) => Err(ApiError(ErrorCode::PasskeyRequired)),
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RulingRegistrationOptions {
    /// 原样传给 `navigator.credentials.create()`。
    #[schema(value_type = Object)]
    pub options: CreationChallengeResponse,
}

/// 开始登记裁决 Passkey。
#[utoipa::path(
    post, path = "/api/v1/arbitration/passkey/register/start", tag = "arbitration",
    responses(
        (status = 200, body = RulingRegistrationOptions),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 429, body = ErrorBody)
    )
)]
pub async fn ruling_register_start(
    State(state): State<AppState>,
    _: RulingOrigin,
    session: Session,
) -> ApiResult<Json<RulingRegistrationOptions>> {
    let user = current_user(&state, &session).await?;
    require_eligible(&state, user.id).await?;
    let username = user
        .username
        .as_deref()
        .ok_or(ApiError(ErrorCode::UsernameRequired))?;
    let existing = db::ruling_passkeys(&state.db, user.id).await?;
    enrollment_allowed(&session, &existing, false).await?;
    limit(&state, &CEREMONY_RATE, user.id).await?;
    let a = &state.arbitration;
    let name = format!("{} 裁决签署 · {username}", a.brand);
    let exclude = existing
        .iter()
        .map(|p| p.credential_id.clone().into())
        .collect::<Vec<_>>();
    let (options, reg) = a
        .webauthn
        .start_passkey_registration(
            a.handles.of(user.id),
            &name,
            &name,
            (!exclude.is_empty()).then_some(exclude),
        )
        .map_err(ApiError::internal)?;
    save_challenge(&state, "reg", user.id, 0, &reg).await?;
    Ok(Json(RulingRegistrationOptions { options }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RulingRegisterFinish {
    /// `navigator.credentials.create()` 的结果。
    #[schema(value_type = Object)]
    pub credential: RegisterPublicKeyCredential,
}

/// 完成登记。成功即视为刚完成一次裁决 Passkey 验证。
#[utoipa::path(
    post, path = "/api/v1/arbitration/passkey/register/finish", tag = "arbitration",
    request_body = RulingRegisterFinish,
    responses(
        (status = 201, body = RulingPasskeyView),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 409, body = ErrorBody)
    )
)]
pub async fn ruling_register_finish(
    State(state): State<AppState>,
    _: RulingOrigin,
    session: Session,
    Json(req): Json<RulingRegisterFinish>,
) -> ApiResult<(StatusCode, Json<RulingPasskeyView>)> {
    let me = current_user_id(&session).await?;
    require_eligible(&state, me).await?;
    let reg: PasskeyRegistration = take_challenge(&state, "reg", me, 0).await?;
    let passkey = state
        .arbitration
        .webauthn
        .finish_passkey_registration(&req.credential, &reg)
        .map_err(|e| {
            tracing::info!(error = %e, "ruling passkey registration rejected");
            ApiError(ErrorCode::PasskeyFailed)
        })?;
    let existing = db::ruling_passkeys(&state.db, me).await?;
    enrollment_allowed(&session, &existing, true).await?;
    let credential = serde_json::to_value(&passkey).map_err(ApiError::internal)?;
    let mut tx = state.db.begin().await?;
    let id = db::insert_ruling_passkey(&mut *tx, me, passkey.cred_id().as_ref(), &credential)
        .await?
        .ok_or(ApiError(ErrorCode::Conflict))?;
    forum_db::audit::append(
        &mut *tx,
        me,
        "arbitration.passkey_added",
        Some(&format!("passkey:{id}")),
        None,
        &json!({ "first": existing.is_empty() }),
    )
    .await?;
    tx.commit().await?;
    mark_passkey_verified(&session).await?;
    let saved = db::ruling_passkeys(&state.db, me)
        .await?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| ApiError::internal("passkey vanished after insert"))?;
    Ok((
        StatusCode::CREATED,
        Json(RulingPasskeyView {
            id: saved.id,
            created_at: saved.created_at,
            last_used_at: saved.last_used_at,
        }),
    ))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RulingAssertionOptions {
    /// 原样传给 `navigator.credentials.get()`。
    #[schema(value_type = Object)]
    pub options: RequestChallengeResponse,
}

/// 用已有的裁决 Passkey 做一次验证（再登记一个 Passkey 之前）。签署裁决另走 `ruling/options`。
#[utoipa::path(
    post, path = "/api/v1/arbitration/passkey/assert/start", tag = "arbitration",
    responses(
        (status = 200, body = RulingAssertionOptions),
        (status = 401, body = ErrorBody), (status = 403, body = ErrorBody), (status = 429, body = ErrorBody)
    )
)]
pub async fn ruling_assert_start(
    State(state): State<AppState>,
    _: RulingOrigin,
    session: Session,
) -> ApiResult<Json<RulingAssertionOptions>> {
    let me = current_user_id(&session).await?;
    require_eligible(&state, me).await?;
    let stored = db::ruling_passkeys(&state.db, me).await?;
    if stored.is_empty() {
        return Err(ApiError(ErrorCode::PasskeyRequired));
    }
    limit(&state, &CEREMONY_RATE, me).await?;
    let creds = stored
        .iter()
        .map(parse_passkey)
        .collect::<ApiResult<Vec<_>>>()?;
    let (options, auth) = state
        .arbitration
        .webauthn
        .start_passkey_authentication(&creds)
        .map_err(ApiError::internal)?;
    save_challenge(&state, "auth", me, 0, &auth).await?;
    Ok(Json(RulingAssertionOptions { options }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RulingAssertFinish {
    /// `navigator.credentials.get()` 的结果。
    #[schema(value_type = Object)]
    pub credential: PublicKeyCredential,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RulingVerified {
    /// 近期验证有效至（Unix 秒）。
    pub verified_until: i64,
}

/// 完成验证：会话获得 5 分钟「近期验证」标记（只用于再登记 Passkey）。
#[utoipa::path(
    post, path = "/api/v1/arbitration/passkey/assert/finish", tag = "arbitration",
    request_body = RulingAssertFinish,
    responses(
        (status = 200, body = RulingVerified),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody)
    )
)]
pub async fn ruling_assert_finish(
    State(state): State<AppState>,
    _: RulingOrigin,
    session: Session,
    Json(req): Json<RulingAssertFinish>,
) -> ApiResult<Json<RulingVerified>> {
    let me = current_user_id(&session).await?;
    require_eligible(&state, me).await?;
    let auth: PasskeyAuthentication = take_challenge(&state, "auth", me, 0).await?;
    verify_assertion(&state, me, &req.credential, &auth).await?;
    Ok(Json(RulingVerified {
        verified_until: mark_passkey_verified(&session).await?,
    }))
}

/// 校验断言并写回签名计数，返回用到的裁决 Passkey ID。凭证必须是本人的裁决 Passkey。
pub(crate) async fn verify_assertion(
    state: &AppState,
    user_id: i64,
    credential: &PublicKeyCredential,
    auth: &PasskeyAuthentication,
) -> ApiResult<i64> {
    let result = state
        .arbitration
        .webauthn
        .finish_passkey_authentication(credential, auth)
        .map_err(|e| {
            tracing::info!(error = %e, "ruling passkey assertion rejected");
            ApiError(ErrorCode::PasskeyFailed)
        })?;
    let stored = db::ruling_passkeys(&state.db, user_id).await?;
    let row = stored
        .iter()
        .find(|p| p.credential_id.as_slice() == result.cred_id().as_ref())
        .ok_or(ApiError(ErrorCode::PasskeyFailed))?;
    let mut passkey = parse_passkey(row)?;
    passkey.update_credential(&result);
    let updated = serde_json::to_value(&passkey).map_err(ApiError::internal)?;
    forum_db::wallet::touch_passkey(&state.db, row.id, &updated).await?;
    Ok(row.id)
}
