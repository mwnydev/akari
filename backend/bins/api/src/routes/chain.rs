//! 链上只读代理（plan 第 2 节「浏览器不直连第三方」）：钱包页面读链上状态都经这里，RPC 服务商密钥只在服务器。
//!
//! 安全与成本决定：
//! - 只放行钱包需要的只读方法（[`METHODS`]）。`eth_call` / `eth_estimateGas` 只能调用本站合约（[`ChainConfig::contracts`]）
//!   或调用者自己的钱包，不接受状态覆盖；`eth_getBlockByNumber` 不返回完整交易。代理不能被当成通用的免费节点。
//! - 只接受钱包页面来源；登录可选。按 IP 限频，已登录再按用户限频。
//! - 常读的结果在 Valkey 里缓存 1–2 秒（只缓存 `latest`），多个页面同时轮询也只打一次节点。`eth_chainId` 直接取链配置。
//! - 节点返回的 JSON-RPC 错误（如 revert）原样转回；节点都不可用时返回 `CHAIN_UNAVAILABLE`。
//!
//! [`ChainConfig::contracts`]: crate::config::ChainConfig::contracts

use std::time::Duration;

use alloy::eips::BlockId;
use alloy::primitives::U256;
use alloy::providers::Provider;
use alloy::rpc::types::{TransactionInput, TransactionRequest};
use alloy::sol_types::SolCall;
use alloy::transports::RpcError;
use axum::Json;
use axum::extract::State;
use forum_chain::ChainError;
use forum_chain::rpc::FailoverRpc;
use forum_core::ErrorCode;
use forum_core::eip712::{Address, B256};
use forum_db::credits;
use forum_kv::{SlidingWindow, ephemeral};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use tower_sessions::Session;
use utoipa::ToSchema;

use super::SESSION_USER_ID;
use super::aa::limit;
use super::me::current_user_id;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::middleware::ClientIp;
use crate::state::AppState;
use crate::wallet::WalletOrigin;

/// 放行的方法；其他一律拒绝。
pub const METHODS: &[&str] = &[
    "eth_chainId",
    "eth_blockNumber",
    "eth_getBalance",
    "eth_getCode",
    "eth_call",
    "eth_getTransactionReceipt",
    "eth_getBlockByNumber",
    "eth_estimateGas",
];

/// 钱包等回执时每秒左右轮询一次，所以给得宽；缓存命中也计数。
const PER_IP: SlidingWindow = SlidingWindow::new("chain-rpc-ip", 900, Duration::from_secs(300));
const PER_USER: SlidingWindow = SlidingWindow::new("chain-rpc-user", 600, Duration::from_secs(300));

const BLOCK_NUMBER_TTL: Duration = Duration::from_secs(1);
const STATE_TTL: Duration = Duration::from_secs(2);
const BALANCES_TTL: Duration = Duration::from_secs(3);

/// `eth_call` / `eth_estimateGas` 交易对象里允许的字段。
const CALL_FIELDS: &[&str] = &["from", "to", "data", "input", "value", "gas"];
const BLOCK_TAGS: &[&str] = &["latest", "pending", "safe", "finalized", "earliest"];

alloy::sol! {
    interface IERC20 {
        function balanceOf(address owner) external view returns (uint256);
    }
}

fn invalid() -> ApiError {
    ApiError(ErrorCode::InvalidParameter)
}

fn unavailable(e: &ChainError) -> ApiError {
    tracing::warn!(error = %e, "chain read failed on all endpoints");
    ApiError(ErrorCode::ChainUnavailable)
}

fn rpc_of(state: &AppState) -> ApiResult<&FailoverRpc> {
    state
        .wallet
        .rpc
        .as_deref()
        .ok_or(ApiError(ErrorCode::ChainUnavailable))
}

/// 检查过的请求：规范化后的参数（缓存键也用它）与缓存时长。
#[derive(Debug)]
struct Checked {
    params: Value,
    ttl: Option<Duration>,
}

fn params_of(v: &Value, max: usize) -> ApiResult<&[Value]> {
    match v {
        Value::Null => Ok(&[]),
        Value::Array(a) if a.len() <= max => Ok(a),
        _ => Err(invalid()),
    }
}

/// 区块标记：缺省为 `latest`；只接受命名标记或 `0x` 区块号。
fn block_tag(v: Option<&Value>) -> ApiResult<String> {
    let Some(v) = v else {
        return Ok("latest".to_owned());
    };
    let s = v.as_str().ok_or_else(invalid)?;
    let hex = s.strip_prefix("0x");
    let is_number = hex.is_some_and(|h| {
        !h.is_empty() && h.len() <= 16 && h.bytes().all(|b| b.is_ascii_hexdigit())
    });
    if BLOCK_TAGS.contains(&s) || is_number {
        Ok(s.to_ascii_lowercase())
    } else {
        Err(invalid())
    }
}

fn address(v: Option<&Value>) -> ApiResult<Address> {
    v.and_then(Value::as_str)
        .and_then(|s| s.parse().ok())
        .ok_or_else(invalid)
}

/// 只有 `latest` 的结果可以缓存几秒；其他标记直接转发。
fn latest_ttl(tag: &str) -> Option<Duration> {
    (tag == "latest").then_some(STATE_TTL)
}

/// 交易对象：字段白名单、全部为十六进制字符串，`to` 必须是本站合约或调用者自己的钱包。
async fn call_object(state: &AppState, user_id: Option<i64>, v: &Value) -> ApiResult<Value> {
    let obj = v.as_object().ok_or_else(invalid)?;
    let mut out = Map::new();
    for (k, v) in obj {
        let s = v.as_str().filter(|_| CALL_FIELDS.contains(&k.as_str()));
        out.insert(
            k.clone(),
            Value::from(s.ok_or_else(invalid)?.to_ascii_lowercase()),
        );
    }
    let to = address(obj.get("to"))?;
    if !state.wallet.chain.contracts.contains(&to) && !is_own_wallet(state, user_id, &to).await? {
        return Err(invalid());
    }
    Ok(Value::Object(out))
}

async fn is_own_wallet(state: &AppState, user_id: Option<i64>, to: &Address) -> ApiResult<bool> {
    let Some(user_id) = user_id else {
        return Ok(false);
    };
    let wallet = forum_db::wallet::wallet_on(&state.db, user_id, state.wallet.chain_id()).await?;
    Ok(wallet.is_some_and(|w| w.address == to.0))
}

async fn check(
    state: &AppState,
    user_id: Option<i64>,
    method: &str,
    params: &Value,
) -> ApiResult<Checked> {
    let checked = match method {
        // 链 ID 直接取链配置，不问节点。
        "eth_chainId" | "eth_blockNumber" => {
            params_of(params, 0)?;
            Checked {
                params: json!([]),
                ttl: (method == "eth_blockNumber").then_some(BLOCK_NUMBER_TTL),
            }
        }
        "eth_getBalance" | "eth_getCode" => {
            let p = params_of(params, 2)?;
            let tag = block_tag(p.get(1))?;
            Checked {
                ttl: latest_ttl(&tag),
                params: json!([address(p.first())?, tag]),
            }
        }
        "eth_call" => {
            // 第 3 个参数是状态覆盖：不接受。
            let p = params_of(params, 2)?;
            let tx = call_object(state, user_id, p.first().ok_or_else(invalid)?).await?;
            let tag = block_tag(p.get(1))?;
            Checked {
                ttl: latest_ttl(&tag),
                params: json!([tx, tag]),
            }
        }
        "eth_estimateGas" => {
            let p = params_of(params, 2)?;
            let tx = call_object(state, user_id, p.first().ok_or_else(invalid)?).await?;
            Checked {
                params: json!([tx, block_tag(p.get(1))?]),
                ttl: None,
            }
        }
        "eth_getTransactionReceipt" => {
            let p = params_of(params, 1)?;
            let hash: B256 = p
                .first()
                .and_then(Value::as_str)
                .and_then(|s| s.parse().ok())
                .ok_or_else(invalid)?;
            Checked {
                params: json!([hash.to_string()]),
                ttl: None,
            }
        }
        "eth_getBlockByNumber" => {
            let p = params_of(params, 2)?;
            if p.get(1).is_some_and(|full| full != &Value::Bool(false)) {
                return Err(invalid());
            }
            Checked {
                params: json!([block_tag(Some(p.first().ok_or_else(invalid)?))?, false]),
                ttl: None,
            }
        }
        _ => return Err(invalid()),
    };
    Ok(checked)
}

/// 节点的回答：`result`，或节点给出的 JSON-RPC `error` 对象。
type Reply = Result<Value, Value>;

async fn forward(rpc: &FailoverRpc, method: &'static str, params: &Value) -> ApiResult<Reply> {
    rpc.call(method, |p| {
        let params = params.clone();
        async move {
            match p.raw_request::<_, Value>(method.into(), params).await {
                Ok(v) => Ok(Ok(v)),
                // 节点正常、只是拒绝或 revert：不切换节点，原样转回。
                Err(RpcError::ErrorResp(e)) => Ok(Err(
                    serde_json::to_value(e).map_err(|e| ChainError::Decode(e.to_string()))?
                )),
                Err(e) => Err(e.into()),
            }
        }
    })
    .await
    .map_err(|e| unavailable(&e))
}

fn cache_key(state: &AppState, method: &str, params: &Value) -> String {
    let digest = Sha256::digest(params.to_string().as_bytes());
    format!(
        "{}chain-rpc:{method}:{}",
        state.cache.ns(),
        hex::encode(digest)
    )
}

/// 单个 JSON-RPC 2.0 请求（不支持批量）。返回 JSON-RPC 响应；参数或目标不在白名单内返回 400。
#[utoipa::path(
    post, path = "/api/v1/chain/rpc", tag = "wallet",
    request_body = Object,
    responses(
        (status = 200, body = Object, description = "JSON-RPC 2.0 响应（成功或节点给出的 error）"),
        (status = 400, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 429, body = ErrorBody), (status = 503, body = ErrorBody)
    )
)]
pub async fn chain_rpc(
    State(state): State<AppState>,
    _origin: WalletOrigin,
    ClientIp(ip): ClientIp,
    session: Session,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    limit(&state, &PER_IP, &ip.to_string()).await?;
    let user_id = session.get::<i64>(SESSION_USER_ID).await?;
    if let Some(id) = user_id {
        limit(&state, &PER_USER, &id.to_string()).await?;
    }
    let id = body.get("id").cloned().unwrap_or(Value::Null);
    let method = body
        .get("method")
        .and_then(Value::as_str)
        .and_then(|m| METHODS.iter().copied().find(|&a| a == m))
        .ok_or_else(invalid)?;
    let req = check(
        &state,
        user_id,
        method,
        body.get("params").unwrap_or(&Value::Null),
    )
    .await?;
    let respond = |r: Reply| {
        Json(match r {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(error) => json!({"jsonrpc": "2.0", "id": id, "error": error}),
        })
    };
    if method == "eth_chainId" {
        return Ok(respond(Ok(json!(format!(
            "{:#x}",
            state.wallet.chain.chain_id
        )))));
    }
    let rpc = rpc_of(&state)?;
    let Some(ttl) = req.ttl else {
        return Ok(respond(forward(rpc, method, &req.params).await?));
    };
    let key = cache_key(&state, method, &req.params);
    if let Some(hit) = ephemeral::get(&state.kv, &key).await? {
        return Ok(respond(Ok(
            serde_json::from_str(&hit).map_err(ApiError::internal)?
        )));
    }
    let reply = forward(rpc, method, &req.params).await?;
    if let Ok(result) = &reply {
        ephemeral::put(&state.kv, &key, &result.to_string(), ttl).await?;
    }
    Ok(respond(reply))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Balances {
    /// USDT 余额（最小单位，十进制字符串）。
    pub usdt: String,
    /// ETH 余额（wei，十进制字符串）。用户钱包平时不需要 ETH，只有站点停摆时自付 gas 才用到。
    pub eth: String,
    /// 读取余额时的区块号。
    pub block: u64,
    /// 剩余免费代付提币次数：开通钱包送 1 次，每完成一笔担保交易（终态且最终确认）得 1 次，每次代付提币用 1 次。
    pub withdraw_credits: i64,
    /// 滚动 24 小时内还能代付提币几次（每天最多 3 次，与剩余次数取小者才是实际可用次数）。
    pub withdraws_left_today: i64,
}

/// 链上余额部分（缓存 3 秒）；提币次数每次从库里读。
#[derive(Debug, Serialize, Deserialize)]
struct OnChain {
    usdt: String,
    eth: String,
    block: u64,
}

async fn read_balances(rpc: &FailoverRpc, token: Address, wallet: Address) -> ApiResult<OnChain> {
    let token = alloy::primitives::Address::from(token.0);
    let wallet = alloy::primitives::Address::from(wallet.0);
    rpc.call("wallet balances", |p| async move {
        let block = p.get_block_number().await?;
        let at = BlockId::number(block);
        let call = TransactionRequest::default()
            .to(token)
            .input(TransactionInput::new(
                IERC20::balanceOfCall { owner: wallet }.abi_encode().into(),
            ));
        let out = p.call(call).block(at).await?;
        let usdt: U256 = IERC20::balanceOfCall::abi_decode_returns(&out)
            .map_err(|e| ChainError::Decode(e.to_string()))?;
        let eth = p.get_balance(wallet).block_id(at).await?;
        Ok(OnChain {
            usdt: usdt.to_string(),
            eth: eth.to_string(),
            block,
        })
    })
    .await
    .map_err(|e| unavailable(&e))
}

/// 我的钱包在本链上的 USDT 与 ETH 余额（同一区块读出，缓存 3 秒），以及免费代付提币次数。
#[utoipa::path(
    get, path = "/api/v1/wallet/balances", tag = "wallet",
    responses(
        (status = 200, body = Balances),
        (status = 401, body = ErrorBody), (status = 403, body = ErrorBody), (status = 503, body = ErrorBody)
    )
)]
pub async fn balances(
    State(state): State<AppState>,
    session: Session,
) -> ApiResult<Json<Balances>> {
    let user_id = current_user_id(&session).await?;
    let wallet = forum_db::wallet::wallet_on(&state.db, user_id, state.wallet.chain_id())
        .await?
        .ok_or(ApiError(ErrorCode::WalletRequired))?;
    let wallet =
        Address(<[u8; 20]>::try_from(wallet.address.as_slice()).map_err(ApiError::internal)?);
    let key = format!("{}chain-balances:{wallet}", state.cache.ns());
    let on_chain = match ephemeral::get(&state.kv, &key).await? {
        Some(hit) => serde_json::from_str(&hit).map_err(ApiError::internal)?,
        None => {
            let rpc = rpc_of(&state)?;
            let b = read_balances(rpc, state.wallet.usdt()?, wallet).await?;
            let json = serde_json::to_string(&b).map_err(ApiError::internal)?;
            ephemeral::put(&state.kv, &key, &json, BALANCES_TTL).await?;
            b
        }
    };
    let since = OffsetDateTime::now_utc() - time::Duration::days(1);
    let credits = credits::usage(&state.db, user_id, since, None).await?;
    Ok(Json(Balances {
        usdt: on_chain.usdt,
        eth: on_chain.eth,
        block: on_chain.block,
        withdraw_credits: credits.balance.max(0),
        withdraws_left_today: (credits::DAILY_LIMIT - credits.today).max(0),
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn block_tags() {
        assert_eq!(block_tag(None).unwrap(), "latest");
        for ok in [
            "latest",
            "pending",
            "safe",
            "finalized",
            "earliest",
            "0x0",
            "0x1A2b",
        ] {
            assert!(block_tag(Some(&json!(ok))).is_ok(), "{ok}");
        }
        assert_eq!(block_tag(Some(&json!("0x1A"))).unwrap(), "0x1a");
        for bad in [
            json!("Latest"),
            json!("0x"),
            json!("12"),
            json!(1),
            json!({"blockHash": "0x00"}),
        ] {
            assert!(block_tag(Some(&bad)).is_err(), "{bad}");
        }
        assert_eq!(latest_ttl("latest"), Some(STATE_TTL));
        assert_eq!(latest_ttl("0x10"), None);
    }

    #[test]
    fn params_must_be_a_short_array() {
        assert!(params_of(&Value::Null, 0).unwrap().is_empty());
        assert_eq!(params_of(&json!([1, 2]), 2).unwrap().len(), 2);
        assert!(params_of(&json!([1, 2, 3]), 2).is_err());
        assert!(params_of(&json!({"a": 1}), 2).is_err());
    }
}
