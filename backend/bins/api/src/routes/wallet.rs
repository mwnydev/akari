//! 开通交易（plan 第 5 节）：邮箱再验证 → 钱包 Passkey 注册 → 登记智能账户地址；以及敏感操作前的 Passkey 再验证。
//!
//! 安全决定：
//! - 所有接口只接受钱包页面的 `Origin`（[`WalletOrigin`]），WebAuthn 本身也只认 `WALLET_ORIGINS`。
//! - 注册**第一个** Passkey 需要 10 分钟内专门为开通交易做过邮箱再验证（一次性，注册成功即消耗），
//!   而不是「登录不满 10 分钟」：被盗的会话无论新旧都开不了钱包，邮件也明确写着「开通交易」，用户能察觉。
//! - 再绑一个 Passkey（第二台设备）需要 5 分钟内用已有的钱包 Passkey 验证过（plan：绑定 Passkey 是敏感操作）。
//! - challenge 存 Valkey 5 分钟，取出即删除，不能重放。

use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use forum_auth::passkey::webauthn::{
    CreationChallengeResponse, CredentialID, Passkey, PasskeyAuthentication, PasskeyRegistration,
    PublicKeyCredential, RegisterPublicKeyCredential, RequestChallengeResponse,
};
use forum_auth::passkey::{P256Key, start_registration};
use forum_chain::kernel::PasskeyPublicKey;
use forum_core::ErrorCode;
use forum_core::eip712::Address;
use forum_db::outbox;
use forum_db::wallet::{self as db, NewPasskey};
use forum_kv::{SlidingWindow, ephemeral};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::OffsetDateTime;
use tower_sessions::Session;
use utoipa::ToSchema;

use super::auth::code_error;
use super::me::current_user_id;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::posting::limit;
use crate::state::AppState;
use crate::views::hex0x;
use crate::wallet::{
    MAX_PASSKEYS, WalletOrigin, email_reverified, mark_email_reverified, mark_passkey_verified,
    passkey_verified_until, recent_passkey, take_email_reverified,
};

pub const KIND_WALLET_CODE: &str = "email.wallet_code";
/// WebAuthn challenge 有效期（plan 第 5 节）。
const CHALLENGE_TTL: Duration = Duration::from_secs(300);

const REVERIFY_RATE: SlidingWindow =
    SlidingWindow::new("wallet-reverify", 5, Duration::from_secs(3600));
const CEREMONY_RATE: SlidingWindow =
    SlidingWindow::new("wallet-ceremony", 30, Duration::from_secs(600));

/// 再验证验证码与登录验证码分开存放：登录码不能拿来开通交易，反之亦然。
fn reverify_subject(email: &str) -> String {
    format!("wallet-reverify\0{email}")
}

fn challenge_key(state: &AppState, kind: &str, user_id: i64) -> String {
    format!("{}wallet:{kind}:{user_id}", state.cache.ns())
}

async fn save_challenge<T: Serialize>(
    state: &AppState,
    kind: &str,
    user_id: i64,
    value: &T,
) -> ApiResult<()> {
    let json = serde_json::to_string(value).map_err(ApiError::internal)?;
    ephemeral::put(
        &state.kv,
        &challenge_key(state, kind, user_id),
        &json,
        CHALLENGE_TTL,
    )
    .await?;
    Ok(())
}

/// 取出并作废 challenge 状态；过期或已用过返回 `PASSKEY_FAILED`。
async fn take_challenge<T: DeserializeOwned>(
    state: &AppState,
    kind: &str,
    user_id: i64,
) -> ApiResult<T> {
    let raw = ephemeral::take(&state.kv, &challenge_key(state, kind, user_id))
        .await?
        .ok_or(ApiError(ErrorCode::PasskeyFailed))?;
    serde_json::from_str(&raw).map_err(ApiError::internal)
}

fn parse_passkey(p: &db::Passkey) -> ApiResult<Passkey> {
    serde_json::from_value(p.credential.clone()).map_err(ApiError::internal)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ReverifyStarted {
    pub expires_in: u64,
    pub resend_in: u64,
}

/// 开通交易前的邮箱再验证：向账号邮箱发一封「开通交易」验证码。
#[utoipa::path(
    post, path = "/api/v1/wallet/reverify/start", tag = "wallet",
    responses(
        (status = 202, body = ReverifyStarted),
        (status = 401, body = ErrorBody), (status = 403, body = ErrorBody), (status = 429, body = ErrorBody)
    )
)]
pub async fn reverify_start(
    State(state): State<AppState>,
    _: WalletOrigin,
    session: Session,
) -> ApiResult<(StatusCode, Json<ReverifyStarted>)> {
    let user = current_user(&state, &session).await?;
    limit(&state, &REVERIFY_RATE, user.id).await?;
    let code = state
        .codes
        .issue(&reverify_subject(&user.email))
        .await
        .map_err(code_error)?;
    outbox::enqueue(
        &state.db,
        KIND_WALLET_CODE,
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

#[derive(Debug, Deserialize, ToSchema)]
pub struct ReverifyRequest {
    pub code: String,
}

/// 校验再验证码；成功后 10 分钟内可注册第一个钱包 Passkey。
#[utoipa::path(
    post, path = "/api/v1/wallet/reverify/verify", tag = "wallet",
    request_body = ReverifyRequest,
    responses(
        (status = 204),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 429, body = ErrorBody)
    )
)]
pub async fn reverify_verify(
    State(state): State<AppState>,
    _: WalletOrigin,
    session: Session,
    Json(req): Json<ReverifyRequest>,
) -> ApiResult<StatusCode> {
    let user = current_user(&state, &session).await?;
    state
        .codes
        .verify(&reverify_subject(&user.email), &req.code)
        .await
        .map_err(code_error)?;
    mark_email_reverified(&session).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn current_user(state: &AppState, session: &Session) -> ApiResult<forum_db::users::User> {
    let id = current_user_id(session).await?;
    forum_db::users::find_by_id(&state.db, id)
        .await?
        .ok_or(ApiError(ErrorCode::Unauthorized))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RegistrationOptions {
    /// 原样传给 `navigator.credentials.create()`（`publicKey` 字段内为 base64url 编码，需先解码）。
    #[schema(value_type = Object)]
    pub options: CreationChallengeResponse,
}

/// 开始注册钱包 Passkey。第一个需要邮箱再验证；之后的需要近期 Passkey 验证。
#[utoipa::path(
    post, path = "/api/v1/wallet/passkey/register/start", tag = "wallet",
    responses(
        (status = 200, body = RegistrationOptions),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 429, body = ErrorBody)
    )
)]
pub async fn register_start(
    State(state): State<AppState>,
    _: WalletOrigin,
    session: Session,
) -> ApiResult<Json<RegistrationOptions>> {
    let user = current_user(&state, &session).await?;
    let username = user
        .username
        .as_deref()
        .ok_or(ApiError(ErrorCode::UsernameRequired))?;
    let existing = db::wallet_passkeys(&state.db, user.id).await?;
    if existing.len() >= MAX_PASSKEYS {
        return Err(ApiError(ErrorCode::TooManyPasskeys));
    }
    if existing.is_empty() {
        if !email_reverified(&session).await? {
            return Err(ApiError(ErrorCode::ReverifyRequired));
        }
    } else {
        recent_passkey(&session).await?;
    }
    limit(&state, &CEREMONY_RATE, user.id).await?;

    let w = &state.wallet;
    let name = format!("{} 钱包签名（勿删）· {username}", w.brand);
    let exclude = existing
        .iter()
        .map(|p| p.credential_id.clone().into())
        .collect();
    let (options, reg) =
        start_registration(&w.webauthn, w.handles.of(user.id), &name, &name, exclude)
            .map_err(ApiError::internal)?;
    save_challenge(&state, "reg", user.id, &reg).await?;
    Ok(Json(RegistrationOptions { options }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RegisterFinish {
    /// `navigator.credentials.create()` 的结果（二进制字段 base64url 编码）。
    #[schema(value_type = Object)]
    pub credential: RegisterPublicKeyCredential,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PasskeyView {
    pub id: i64,
    /// base64url 凭证 ID。
    pub credential_id: String,
    /// P-256 公钥 x 坐标（`0x` + 64 位十六进制），用于计算 Kernel 智能账户地址。
    pub pubkey_x: String,
    pub pubkey_y: String,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[schema(value_type = Option<String>, format = DateTime)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_used_at: Option<OffsetDateTime>,
}

impl From<&db::Passkey> for PasskeyView {
    fn from(p: &db::Passkey) -> Self {
        Self {
            id: p.id,
            credential_id: base64url(&p.credential_id),
            pubkey_x: hex0x(&p.pubkey_x),
            pubkey_y: hex0x(&p.pubkey_y),
            created_at: p.created_at,
            last_used_at: p.last_used_at,
        }
    }
}

/// 与 WebAuthn JSON 相同的 base64url 编码（借用 webauthn-rs 的序列化，避免再引一个编码库）。
fn base64url(b: &[u8]) -> String {
    serde_json::to_value(CredentialID::from(b.to_vec()))
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// 完成注册：保存凭证 ID、COSE 公钥与 P-256 坐标。成功即视为刚完成一次 Passkey 验证。
#[utoipa::path(
    post, path = "/api/v1/wallet/passkey/register/finish", tag = "wallet",
    request_body = RegisterFinish,
    responses(
        (status = 201, body = PasskeyView),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 409, body = ErrorBody)
    )
)]
pub async fn register_finish(
    State(state): State<AppState>,
    _: WalletOrigin,
    session: Session,
    Json(req): Json<RegisterFinish>,
) -> ApiResult<(StatusCode, Json<PasskeyView>)> {
    let user_id = current_user_id(&session).await?;
    let reg: PasskeyRegistration = take_challenge(&state, "reg", user_id).await?;
    let passkey = state
        .wallet
        .webauthn
        .finish_passkey_registration(&req.credential, &reg)
        .map_err(|e| {
            tracing::info!(error = %e, "passkey registration rejected");
            ApiError(ErrorCode::PasskeyFailed)
        })?;
    let key = P256Key::of(&passkey).ok_or(ApiError(ErrorCode::PasskeyFailed))?;
    // 开始注册后才检查的条件在这里复核：第一个 Passkey 消耗邮箱再验证标记。
    let existing = db::wallet_passkeys(&state.db, user_id).await?;
    if existing.len() >= MAX_PASSKEYS {
        return Err(ApiError(ErrorCode::TooManyPasskeys));
    }
    if existing.is_empty() {
        if !take_email_reverified(&session).await? {
            return Err(ApiError(ErrorCode::ReverifyRequired));
        }
    } else {
        recent_passkey(&session).await?;
    }
    let credential = serde_json::to_value(&passkey).map_err(ApiError::internal)?;
    let cose = key.cose();
    let id = db::insert_passkey(
        &state.db,
        NewPasskey {
            user_id,
            credential_id: passkey.cred_id().as_ref(),
            credential: &credential,
            public_key_cose: &cose,
            pubkey_x: &key.x,
            pubkey_y: &key.y,
        },
    )
    .await?
    .ok_or(ApiError(ErrorCode::Conflict))?;
    mark_passkey_verified(&session, id).await?;
    let saved = db::wallet_passkeys(&state.db, user_id)
        .await?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| ApiError::internal("passkey vanished after insert"))?;
    Ok((StatusCode::CREATED, Json(PasskeyView::from(&saved))))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AssertionOptions {
    /// 原样传给 `navigator.credentials.get()`。
    #[schema(value_type = Object)]
    pub options: RequestChallengeResponse,
}

/// 开始 Passkey 再验证（敏感操作前：私信发起担保、绑定 Passkey、登记钱包地址等）。
#[utoipa::path(
    post, path = "/api/v1/wallet/passkey/assert/start", tag = "wallet",
    responses(
        (status = 200, body = AssertionOptions),
        (status = 401, body = ErrorBody), (status = 403, body = ErrorBody), (status = 429, body = ErrorBody)
    )
)]
pub async fn assert_start(
    State(state): State<AppState>,
    _: WalletOrigin,
    session: Session,
) -> ApiResult<Json<AssertionOptions>> {
    let user_id = current_user_id(&session).await?;
    let stored = db::wallet_passkeys(&state.db, user_id).await?;
    if stored.is_empty() {
        return Err(ApiError(ErrorCode::WalletRequired));
    }
    limit(&state, &CEREMONY_RATE, user_id).await?;
    let creds = stored
        .iter()
        .map(parse_passkey)
        .collect::<ApiResult<Vec<_>>>()?;
    let (options, auth) = state
        .wallet
        .webauthn
        .start_passkey_authentication(&creds)
        .map_err(ApiError::internal)?;
    save_challenge(&state, "auth", user_id, &auth).await?;
    Ok(Json(AssertionOptions { options }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AssertFinish {
    /// `navigator.credentials.get()` 的结果。
    #[schema(value_type = Object)]
    pub credential: PublicKeyCredential,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Verified {
    pub passkey_id: i64,
    /// 近期验证有效至（Unix 秒）。
    pub verified_until: i64,
}

/// 完成 Passkey 再验证：会话获得 5 分钟「近期验证」标记。
#[utoipa::path(
    post, path = "/api/v1/wallet/passkey/assert/finish", tag = "wallet",
    request_body = AssertFinish,
    responses(
        (status = 200, body = Verified),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody)
    )
)]
pub async fn assert_finish(
    State(state): State<AppState>,
    _: WalletOrigin,
    session: Session,
    Json(req): Json<AssertFinish>,
) -> ApiResult<Json<Verified>> {
    let user_id = current_user_id(&session).await?;
    let auth: PasskeyAuthentication = take_challenge(&state, "auth", user_id).await?;
    let result = state
        .wallet
        .webauthn
        .finish_passkey_authentication(&req.credential, &auth)
        .map_err(|e| {
            tracing::info!(error = %e, "passkey assertion rejected");
            ApiError(ErrorCode::PasskeyFailed)
        })?;
    let stored = db::wallet_passkeys(&state.db, user_id).await?;
    let row = stored
        .iter()
        .find(|p| p.credential_id.as_slice() == result.cred_id().as_ref())
        .ok_or(ApiError(ErrorCode::PasskeyFailed))?;
    let mut passkey = parse_passkey(row)?;
    passkey.update_credential(&result);
    let credential = serde_json::to_value(&passkey).map_err(ApiError::internal)?;
    db::touch_passkey(&state.db, row.id, &credential).await?;
    let verified_until = mark_passkey_verified(&session, row.id).await?;
    Ok(Json(Verified {
        passkey_id: row.id,
        verified_until,
    }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewWallet {
    pub chain_id: u64,
    /// 浏览器用 ZeroDev SDK（Kernel v3.3 + EntryPoint v0.7）按 `init_version` 的参数算出的 counterfactual 地址。
    #[schema(value_type = String, example = "0x1111111111111111111111111111111111111111")]
    pub address: Address,
    /// 计算地址用的初始化参数版本（`GET /api/v1/trade/params` 的 `wallet_init_versions`）。
    pub init_version: u32,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WalletView {
    pub chain_id: i64,
    pub address: String,
    pub passkey_id: i64,
    /// 初始化参数版本：推算地址、部署钱包都用这个版本，不用当前最新版本（ADR 0008）。
    pub init_version: i32,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

impl From<db::Wallet> for WalletView {
    fn from(w: db::Wallet) -> Self {
        Self {
            chain_id: w.chain_id,
            address: hex0x(&w.address),
            passkey_id: w.passkey_id,
            init_version: w.init_version,
            created_at: w.created_at,
        }
    }
}

/// 登记智能账户地址，绑定到 5 分钟内刚验证过的那个 Passkey（通常是刚注册的那个）。每人每条链一个。
/// 服务端按 `init_version` 的参数与该 Passkey 公钥推算地址，与提交的不一致时拒绝。
#[utoipa::path(
    post, path = "/api/v1/wallet/account", tag = "wallet",
    request_body = NewWallet,
    responses(
        (status = 201, body = WalletView),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 409, body = ErrorBody)
    )
)]
pub async fn create_account(
    State(state): State<AppState>,
    _: WalletOrigin,
    session: Session,
    Json(req): Json<NewWallet>,
) -> ApiResult<(StatusCode, Json<WalletView>)> {
    let user_id = current_user_id(&session).await?;
    let passkey_id = recent_passkey(&session).await?;
    let w = &state.wallet;
    if req.chain_id != w.chain.chain_id || req.address.is_zero() {
        return Err(ApiError(ErrorCode::InvalidParameter));
    }
    let version = w
        .chain
        .wallet_init_version(req.init_version)
        .ok_or(ApiError(ErrorCode::InvalidParameter))?;
    let init_version =
        i32::try_from(version.version).map_err(|_| ApiError(ErrorCode::InvalidParameter))?;
    let passkey = db::wallet_passkeys(&state.db, user_id)
        .await?
        .into_iter()
        .find(|p| p.id == passkey_id)
        .ok_or(ApiError(ErrorCode::PasskeyRequired))?;
    let coordinate = |c: &[u8]| <[u8; 32]>::try_from(c).map_err(ApiError::internal);
    let key = PasskeyPublicKey::new(
        coordinate(&passkey.pubkey_x)?,
        coordinate(&passkey.pubkey_y)?,
        &passkey.credential_id,
    );
    if !w
        .verifier
        .verify(version, &req.address, &key)
        .await
        .map_err(ApiError::internal)?
    {
        return Err(ApiError(ErrorCode::WalletAddressMismatch));
    }
    let wallet = db::insert_wallet(
        &state.db,
        db::NewWallet {
            user_id,
            chain_id: w.chain_id(),
            address: &req.address.0,
            passkey_id,
            init_version,
        },
    )
    .await?
    .ok_or(ApiError(ErrorCode::Conflict))?;
    Ok((StatusCode::CREATED, Json(wallet.into())))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WalletOverview {
    /// 是否已开通交易（至少一个钱包 Passkey 和本链钱包地址）。
    pub enabled: bool,
    pub passkeys: Vec<PasskeyView>,
    pub wallets: Vec<WalletView>,
    /// 当前会话的 Passkey 近期验证截止时间（Unix 秒）。
    pub verified_until: Option<i64>,
}

/// 我的钱包 Passkey 与地址。
#[utoipa::path(
    get, path = "/api/v1/wallet", tag = "wallet",
    responses((status = 200, body = WalletOverview), (status = 401, body = ErrorBody))
)]
pub async fn overview(
    State(state): State<AppState>,
    session: Session,
) -> ApiResult<Json<WalletOverview>> {
    let user_id = current_user_id(&session).await?;
    let passkeys = db::wallet_passkeys(&state.db, user_id).await?;
    let wallets = db::wallets(&state.db, user_id).await?;
    let chain_id = state.wallet.chain_id();
    Ok(Json(WalletOverview {
        enabled: !passkeys.is_empty() && wallets.iter().any(|w| w.chain_id == chain_id),
        passkeys: passkeys.iter().map(PasskeyView::from).collect(),
        wallets: wallets.into_iter().map(WalletView::from).collect(),
        verified_until: passkey_verified_until(&session).await?,
    }))
}
