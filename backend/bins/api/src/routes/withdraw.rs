//! 提币地址（plan 第 5 节「提币地址冷却」「提回交易所」）。
//!
//! 安全决定：
//! - 添加、撤销都是敏感操作：只接受钱包页面来源，且 5 分钟内用钱包 Passkey 验证过。
//! - 第一个地址立即生效；之后添加的 24 小时后生效，期间可撤销。防的是会话被盗后偷换地址：forum-sponsor
//!   只代付转到已生效地址的 USDT（真正动钱仍需钱包 Passkey 签名）。
//! - 添加后发邮件与站内通知，写明生效时间，本人不知情时能在冷却期内撤销。
//! - 不能添加本人钱包地址与本站合约地址（转进去的币取不回或毫无意义）。

use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use forum_core::ErrorCode;
use forum_core::eip712::Address;
use forum_db::outbox;
use forum_db::withdraw::{self as db, MAX_LIVE, NewWithdrawAddress, WithdrawAddress};
use forum_kv::SlidingWindow;
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tower_sessions::Session;
use utoipa::ToSchema;

use super::me::current_user_id;
use super::wallet::current_user;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::posting::limit;
use crate::state::AppState;
use crate::views::hex0x;
use crate::wallet::{WalletOrigin, recent_passkey};

pub const KIND_ADDED_EMAIL: &str = "email.withdraw_address_added";
pub const KIND_ADDED_NOTICE: &str = "notify.withdraw_address_added";
/// 新地址的冷却期（plan 第 9 节「账号安全」：提币地址冷却 24 小时）。
pub const COOLDOWN_SECS: i64 = 86_400;
pub const MAX_LABEL_CHARS: usize = 32;

const ADD_RATE: SlidingWindow =
    SlidingWindow::new("withdraw-address-add", 20, Duration::from_secs(3600));

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum WithdrawAddressStatus {
    Active,
    /// 冷却中，`active_at` 后生效。
    Pending,
    Revoked,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WithdrawAddressView {
    pub id: i64,
    pub label: String,
    /// `0x` + 40 位小写十六进制。
    pub address: String,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub active_at: OffsetDateTime,
    pub status: WithdrawAddressStatus,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

impl From<WithdrawAddress> for WithdrawAddressView {
    fn from(a: WithdrawAddress) -> Self {
        let status = if a.revoked_at.is_some() {
            WithdrawAddressStatus::Revoked
        } else if a.active {
            WithdrawAddressStatus::Active
        } else {
            WithdrawAddressStatus::Pending
        };
        Self {
            id: a.id,
            label: a.label,
            address: hex0x(&a.address),
            active_at: a.active_at,
            status,
            created_at: a.created_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WithdrawAddressList {
    /// 未撤销的在前，再按添加时间倒序；已撤销的只返回最近的一部分。
    pub addresses: Vec<WithdrawAddressView>,
}

/// 我在本链上的提币地址。
#[utoipa::path(
    get, path = "/api/v1/wallet/withdraw-addresses", tag = "wallet",
    responses((status = 200, body = WithdrawAddressList), (status = 401, body = ErrorBody))
)]
pub async fn list_withdraw_addresses(
    State(state): State<AppState>,
    session: Session,
) -> ApiResult<Json<WithdrawAddressList>> {
    let user_id = current_user_id(&session).await?;
    let rows = db::list(&state.db, user_id, state.wallet.chain_id()).await?;
    Ok(Json(WithdrawAddressList {
        addresses: rows.into_iter().map(Into::into).collect(),
    }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewWithdrawAddressRequest {
    /// 备注（如交易所名），最多 32 个字，可为空。
    pub label: String,
    /// 交易所的 Arbitrum One 充值地址。大小写混合时按 EIP-55 校验和检查，防止粘贴错一位。
    #[schema(example = "0x1111111111111111111111111111111111111111")]
    pub address: String,
}

/// `0x` + 40 位十六进制；大小写混合的必须是正确的 EIP-55 校验和。
fn parse_address(s: &str) -> ApiResult<Address> {
    let invalid = || ApiError(ErrorCode::WithdrawAddressInvalid);
    let s = s.trim();
    let hex = s.strip_prefix("0x").ok_or_else(invalid)?;
    if hex.len() != 40 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid());
    }
    let mixed_case =
        hex.bytes().any(|b| b.is_ascii_lowercase()) && hex.bytes().any(|b| b.is_ascii_uppercase());
    let parsed = if mixed_case {
        alloy::primitives::Address::parse_checksummed(s, None).map_err(|_| invalid())?
    } else {
        s.parse().map_err(|_| invalid())?
    };
    Ok(Address(parsed.into_array()))
}

fn parse_label(s: &str) -> ApiResult<&str> {
    let label = s.trim();
    if label.chars().count() > MAX_LABEL_CHARS || label.chars().any(char::is_control) {
        return Err(ApiError(ErrorCode::InvalidParameter));
    }
    Ok(label)
}

/// 添加提币地址。本人第一个地址立即生效，之后添加的 24 小时后生效。需要 5 分钟内的钱包 Passkey 验证。
#[utoipa::path(
    post, path = "/api/v1/wallet/withdraw-addresses", tag = "wallet",
    request_body = NewWithdrawAddressRequest,
    responses(
        (status = 201, body = WithdrawAddressView),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 409, body = ErrorBody), (status = 429, body = ErrorBody)
    )
)]
pub async fn add_withdraw_address(
    State(state): State<AppState>,
    _: WalletOrigin,
    session: Session,
    Json(req): Json<NewWithdrawAddressRequest>,
) -> ApiResult<(StatusCode, Json<WithdrawAddressView>)> {
    let user = current_user(&state, &session).await?;
    recent_passkey(&session).await?;
    let label = parse_label(&req.label)?;
    let address = parse_address(&req.address)?;
    let chain_id = state.wallet.chain_id();
    let own_wallet = forum_db::wallet::wallet_on(&state.db, user.id, chain_id)
        .await?
        .is_some_and(|w| w.address == address.0);
    if address.is_zero() || own_wallet || state.wallet.chain.contracts.contains(&address) {
        return Err(ApiError(ErrorCode::WithdrawAddressForbidden));
    }
    limit(&state, &ADD_RATE, user.id).await?;

    let mut tx = state.db.begin().await?;
    db::lock_user(&mut tx, user.id).await?;
    let stats = db::stats(&mut *tx, user.id, chain_id, &address.0).await?;
    if stats.duplicate {
        return Err(ApiError(ErrorCode::Conflict));
    }
    if stats.live >= MAX_LIVE {
        return Err(ApiError(ErrorCode::WithdrawAddressLimit));
    }
    let saved = db::insert(
        &mut *tx,
        NewWithdrawAddress {
            user_id: user.id,
            chain_id,
            address: &address.0,
            label,
            cooldown_secs: if stats.used_before { COOLDOWN_SECS } else { 0 },
        },
    )
    .await?;
    let shown = hex0x(&saved.address);
    // 邮件不带备注：备注是用户输入，不进邮件 HTML。
    outbox::enqueue(
        &mut *tx,
        KIND_ADDED_EMAIL,
        &json!({
            "to": user.email,
            "address": shown,
            "active_at": forum_core::email::display_time(saved.active_at),
            "immediate": saved.active,
        }),
        outbox::PRIORITY_HIGH,
    )
    .await?;
    outbox::enqueue(
        &mut *tx,
        KIND_ADDED_NOTICE,
        &json!({
            "to": user.id,
            "withdraw_address_id": saved.id,
            "address": shown,
            "active_at": saved.active_at.format(&Rfc3339).map_err(ApiError::internal)?,
        }),
        outbox::PRIORITY_NORMAL,
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(saved.into())))
}

/// 撤销提币地址（冷却中或已生效的都可以）。需要 5 分钟内的钱包 Passkey 验证。
#[utoipa::path(
    delete, path = "/api/v1/wallet/withdraw-addresses/{id}", tag = "wallet",
    params(("id" = i64, Path)),
    responses(
        (status = 204),
        (status = 401, body = ErrorBody), (status = 403, body = ErrorBody), (status = 404, body = ErrorBody)
    )
)]
pub async fn revoke_withdraw_address(
    State(state): State<AppState>,
    _: WalletOrigin,
    session: Session,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let user_id = current_user_id(&session).await?;
    recent_passkey(&session).await?;
    if db::revoke(&state.db, user_id, id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError(ErrorCode::NotFound))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn addresses_are_checked() {
        let lower = "0x52908400098527886e0f7030069857d2e4169ee7";
        assert_eq!(parse_address(lower).unwrap().to_string(), lower);
        assert_eq!(
            parse_address(&format!("  {}\n", lower.to_uppercase().replace("0X", "0x"))).unwrap(),
            parse_address(lower).unwrap()
        );
        // EIP-55 正确的校验和可以，改一个字母的大小写就不行。
        let checksummed = "0x52908400098527886E0F7030069857D2E4169EE7";
        assert!(parse_address(checksummed).is_ok());
        let typo = "0x52908400098527886E0F7030069857D2E4169Ee7";
        assert_eq!(
            parse_address(typo).unwrap_err().0,
            ErrorCode::WithdrawAddressInvalid
        );
        for bad in [
            "52908400098527886e0f7030069857d2e4169ee7",
            "0x1234",
            "0xzz08400098527886e0f7030069857d2e4169ee7",
        ] {
            assert!(parse_address(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn labels_are_short_plain_text() {
        assert_eq!(parse_label("  币安 Binance ").unwrap(), "币安 Binance");
        assert_eq!(parse_label("").unwrap(), "");
        assert!(parse_label(&"字".repeat(32)).is_ok());
        assert!(parse_label(&"字".repeat(33)).is_err());
        assert!(parse_label("a\nb").is_err());
    }
}
