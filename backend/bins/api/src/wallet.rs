//! 钱包相关的共享状态：Webauthn 实例、钱包来源校验、会话里的「近期验证」标记、链参数。

use std::sync::Arc;

use axum::extract::{FromRef, FromRequestParts};
use axum::http::header;
use axum::http::request::Parts;
use forum_auth::passkey::{UserHandles, webauthn::Webauthn};
use forum_chain::config::WalletInitVersion;
use forum_chain::kernel::{AddressResolver, PasskeyPublicKey};
use forum_core::ErrorCode;
use forum_core::eip712::{Address, Domain};
use forum_core::trade::TradeLimits;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tower_sessions::Session;

use crate::config::ChainConfig;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// 邮箱再验证有效 10 分钟（与验证码有效期一致）：只用于开通交易时注册第一个钱包 Passkey。
pub const EMAIL_REVERIFY_SECS: i64 = 600;
/// Passkey 再验证有效 5 分钟（plan 第 9 节「敏感操作再验证有效 5 分钟」）。
pub const PASSKEY_RECENT_SECS: i64 = 300;
/// 每人最多登记的钱包 Passkey 数（本机 + 第二台设备，留余量）。
pub const MAX_PASSKEYS: usize = 5;

const SESSION_EMAIL_REVERIFIED: &str = "wallet_email_at";
const SESSION_PASSKEY_VERIFIED: &str = "wallet_passkey";

/// 浏览器算出的钱包地址能否按该初始化参数版本由这个 Passkey 推出（ADR 0008）。
/// 生产实现是 [`ChainAddressVerifier`]；测试用假实现，不连链。
#[async_trait::async_trait]
pub trait AddressVerifier: Send + Sync {
    async fn verify(
        &self,
        version: &WalletInitVersion,
        address: &Address,
        key: &PasskeyPublicKey,
    ) -> anyhow::Result<bool>;
}

/// 经链上 Kernel 工厂 `getAddress`（eth_call，主备节点）推算 counterfactual 地址再比较。
pub struct ChainAddressVerifier(pub AddressResolver);

#[async_trait::async_trait]
impl AddressVerifier for ChainAddressVerifier {
    async fn verify(
        &self,
        version: &WalletInitVersion,
        address: &Address,
        key: &PasskeyPublicKey,
    ) -> anyhow::Result<bool> {
        Ok(self.0.address(version, key).await?.0 == address.0)
    }
}

/// 没有链配置（`CHAIN_CONFIG`）时：没有初始化参数版本，登记钱包在查版本时就被拒绝，走不到这里。
pub struct NoChainVerifier;

#[async_trait::async_trait]
impl AddressVerifier for NoChainVerifier {
    async fn verify(
        &self,
        _: &WalletInitVersion,
        _: &Address,
        _: &PasskeyPublicKey,
    ) -> anyhow::Result<bool> {
        anyhow::bail!("wallet address verification needs CHAIN_CONFIG")
    }
}

pub struct WalletState {
    pub webauthn: Webauthn,
    /// 钱包页面的 Origin（与请求头逐字比较）。
    pub origins: Vec<String>,
    pub brand: String,
    pub handles: UserHandles,
    pub chain: ChainConfig,
    pub limits: TradeLimits,
    pub verifier: Arc<dyn AddressVerifier>,
}

impl WalletState {
    /// 报价签名域；合约地址未配置时报价功能不可用。
    pub fn domain(&self) -> ApiResult<Domain> {
        let verifying_contract = self
            .chain
            .escrow
            .ok_or_else(|| ApiError::internal("ESCROW_ADDRESS not configured"))?;
        Ok(Domain {
            chain_id: self.chain.chain_id,
            verifying_contract,
        })
    }

    pub fn usdt(&self) -> ApiResult<Address> {
        self.chain
            .usdt
            .ok_or_else(|| ApiError::internal("USDT_ADDRESS not configured"))
    }

    /// `i64` 形式的链 ID（数据库列类型）。
    pub fn chain_id(&self) -> i64 {
        i64::try_from(self.chain.chain_id).unwrap_or(i64::MAX)
    }
}

/// 只接受来自钱包页面的请求：钱包 Passkey 的 challenge 与绑定操作不能由论坛主站发起（plan 第 5、11 节）。
/// 浏览器在跨源请求与所有 POST 上都会带 `Origin`，缺失即拒绝。
#[derive(Debug, Clone, Copy)]
pub struct WalletOrigin;

impl<S> FromRequestParts<S> for WalletOrigin
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
            .is_some_and(|o| state.wallet.origins.iter().any(|a| a == o));
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

/// `at` 起 `secs` 秒内。
fn within(at: Option<i64>, secs: i64) -> bool {
    at.is_some_and(|t| now() - t <= secs)
}

/// 刚完成开通交易用的邮箱再验证。
pub async fn mark_email_reverified(session: &Session) -> ApiResult<()> {
    session.insert(SESSION_EMAIL_REVERIFIED, now()).await?;
    Ok(())
}

/// 消耗邮箱再验证标记：10 分钟内有效、只能用一次。
pub async fn take_email_reverified(session: &Session) -> ApiResult<bool> {
    let at = session.remove::<i64>(SESSION_EMAIL_REVERIFIED).await?;
    Ok(within(at, EMAIL_REVERIFY_SECS))
}

pub async fn email_reverified(session: &Session) -> ApiResult<bool> {
    let at = session.get::<i64>(SESSION_EMAIL_REVERIFIED).await?;
    Ok(within(at, EMAIL_REVERIFY_SECS))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct PasskeyVerified {
    at: i64,
    passkey_id: i64,
}

/// 刚用某个钱包 Passkey 完成用户验证（注册或断言）。返回有效期截止时间（Unix 秒）。
pub async fn mark_passkey_verified(session: &Session, passkey_id: i64) -> ApiResult<i64> {
    let at = now();
    session
        .insert(SESSION_PASSKEY_VERIFIED, PasskeyVerified { at, passkey_id })
        .await?;
    Ok(at + PASSKEY_RECENT_SECS)
}

/// 5 分钟内用过的钱包 Passkey；没有返回 `PASSKEY_REQUIRED`。
pub async fn recent_passkey(session: &Session) -> ApiResult<i64> {
    session
        .get::<PasskeyVerified>(SESSION_PASSKEY_VERIFIED)
        .await?
        .filter(|v| within(Some(v.at), PASSKEY_RECENT_SECS))
        .map(|v| v.passkey_id)
        .ok_or(ApiError(ErrorCode::PasskeyRequired))
}

/// 近期验证的截止时间（展示用）。
pub async fn passkey_verified_until(session: &Session) -> ApiResult<Option<i64>> {
    Ok(session
        .get::<PasskeyVerified>(SESSION_PASSKEY_VERIFIED)
        .await?
        .map(|v| v.at + PASSKEY_RECENT_SECS)
        .filter(|&until| until >= now()))
}
