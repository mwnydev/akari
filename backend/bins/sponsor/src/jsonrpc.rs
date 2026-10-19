//! JSON-RPC 2.0 信封与错误。
//!
//! 业务拒绝用 ERC-4337 / ERC-7677 约定的错误码，`data.code` 带 [`ErrorCode`]（前端据此显示中文提示），
//! `data.reason` 是给开发者看的简短原因（不含任何密钥或内部地址以外的信息）。

use forum_core::ErrorCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    #[serde(default)]
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL: i64 = -32603;
/// ERC-4337：Paymaster 拒绝。
pub const PAYMASTER_REJECTED: i64 = -32501;
/// ERC-4337：被限流 / 封禁。
pub const THROTTLED: i64 = -32504;
/// ERC-4337：执行阶段会回滚。
pub const EXECUTION_REVERTED: i64 = -32521;

/// 一次被拒绝的请求。`reason` 只用于排查，不展示给用户。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {reason}")]
pub struct Reject {
    pub code: ErrorCode,
    pub reason: String,
}

impl Reject {
    pub fn new(code: ErrorCode, reason: impl Into<String>) -> Self {
        Self {
            code,
            reason: reason.into(),
        }
    }

    pub fn not_allowed(reason: impl Into<String>) -> Self {
        Self::new(ErrorCode::SponsorNotAllowed, reason)
    }

    pub fn invalid(reason: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidParameter, reason)
    }
}

/// 服务内部的错误：拒绝（告诉调用方原因）或内部故障（只记日志）。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Reject(#[from] Reject),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl From<sqlx::Error> for Error {
    fn from(e: sqlx::Error) -> Self {
        Self::Internal(e.into())
    }
}

impl From<forum_chain::ChainError> for Error {
    fn from(e: forum_chain::ChainError) -> Self {
        Self::Internal(e.into())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

fn rpc_code(code: ErrorCode) -> i64 {
    match code {
        ErrorCode::InvalidParameter => INVALID_PARAMS,
        ErrorCode::SponsorNotAllowed
        | ErrorCode::WalletRequired
        | ErrorCode::WithdrawAddressNotActive
        | ErrorCode::WithdrawCreditRequired => PAYMASTER_REJECTED,
        ErrorCode::SponsorQuotaExceeded
        | ErrorCode::SponsorPending
        | ErrorCode::SponsorSuspended
        | ErrorCode::RateLimited => THROTTLED,
        ErrorCode::SimulationFailed => EXECUTION_REVERTED,
        _ => INTERNAL,
    }
}

impl From<Error> for RpcError {
    fn from(e: Error) -> Self {
        match e {
            Error::Reject(r) => Self {
                code: rpc_code(r.code),
                message: r.code.message().to_owned(),
                data: Some(json!({"code": r.code, "reason": r.reason})),
            },
            Error::Internal(err) => {
                tracing::error!(error = %err, "internal error");
                Self {
                    code: INTERNAL,
                    message: ErrorCode::Internal.message().to_owned(),
                    data: Some(json!({"code": ErrorCode::Internal})),
                }
            }
        }
    }
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
}

/// 组装响应：`result` 与 `error` 二选一。
pub fn response(id: Value, result: std::result::Result<Value, RpcError>) -> Value {
    match result {
        Ok(v) => json!({"jsonrpc": "2.0", "id": id, "result": v}),
        Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": e}),
    }
}

/// 按位置取参数并反序列化。
pub fn param<T: serde::de::DeserializeOwned>(params: &Value, i: usize, name: &str) -> Result<T> {
    let v = params.get(i).cloned().unwrap_or(Value::Null);
    serde_json::from_value(v).map_err(|e| Reject::invalid(format!("{name}: {e}")).into())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn rejects_carry_the_error_code() {
        let e: RpcError = Error::from(Reject::new(ErrorCode::SponsorPending, "nonce 3")).into();
        assert_eq!(e.code, THROTTLED);
        assert_eq!(e.message, ErrorCode::SponsorPending.message());
        assert_eq!(e.data.unwrap()["code"], "SPONSOR_PENDING");
    }

    #[test]
    fn internal_errors_hide_details() {
        let e: RpcError = Error::Internal(anyhow::anyhow!("db password wrong")).into();
        assert_eq!(e.code, INTERNAL);
        assert!(!serde_json::to_string(&e).unwrap().contains("password"));
    }

    #[test]
    fn params_are_positional() {
        let p = json!(["0x1", 2]);
        let n: u64 = param(&p, 1, "n").unwrap();
        assert_eq!(n, 2);
        assert!(param::<u64>(&p, 0, "n").is_err());
        assert!(param::<u64>(&p, 5, "n").is_err());
    }
}
