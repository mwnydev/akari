//! JSON-RPC 方法分发：ERC-7677 Paymaster 方法 + ERC-4337 bundler 方法子集。
//!
//! 只代发经本服务签过代付的操作（用户钱包里没有 ETH，没有别的可能）。私有代发器不健康时，
//! 估算、报价与提交转交第三方 bundler（`FALLBACK_BUNDLER_URL`），gas 仍由自有 Paymaster 付；
//! 查询先看本地记录，没有再问第三方。

use std::sync::Arc;

use alloy::primitives::{Address, B256, U64};
use alloy::rpc::types::{Log, TransactionReceipt};
use alloy::sol_types::SolEvent;
use alloy::transports::http::reqwest;
use forum_chain::aa::{IEntryPoint, SponsorTerms, UserOperation};
use forum_db::sponsor::{self as db, SponsorshipStatus};
use serde_json::{Value, json};

use crate::jsonrpc::{
    self, INTERNAL, INVALID_REQUEST, METHOD_NOT_FOUND, Reject, Request, Result, RpcError, param,
};
use crate::relayer::{Queued, Relayer};
use crate::sim;
use crate::sponsor::Sponsor;

/// 第三方 bundler（故障备用）。地址带服务商密钥，日志里只出现主机名。
pub struct Fallback {
    client: reqwest::Client,
    url: reqwest::Url,
}

impl Fallback {
    pub fn new(url: &str) -> anyhow::Result<Self> {
        let url: reqwest::Url = url.parse()?;
        Ok(Self {
            client: forum_chain::rpc::http_client(&url)?,
            url,
        })
    }

    pub async fn request(
        &self,
        method: &str,
        params: Value,
    ) -> std::result::Result<Value, RpcError> {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        let unavailable = |e: &dyn std::fmt::Display| {
            tracing::warn!(
                bundler = %forum_chain::rpc::redact(&self.url),
                method,
                error = %e,
                "fallback bundler request failed"
            );
            RpcError::new(
                INTERNAL,
                forum_core::ErrorCode::SponsorUnavailable.message(),
            )
        };
        let res = self
            .client
            .post(self.url.clone())
            .json(&body)
            .send()
            .await
            .map_err(|e| unavailable(&e))?;
        let mut v: Value = res.json().await.map_err(|e| unavailable(&e))?;
        if let Some(err) = v.get_mut("error") {
            return Err(serde_json::from_value(err.take()).map_err(|e| unavailable(&e))?);
        }
        Ok(v.get_mut("result").map(Value::take).unwrap_or(Value::Null))
    }
}

pub struct Bundler {
    pub sponsor: Arc<Sponsor>,
    pub relayer: Relayer,
    pub fallback: Option<Fallback>,
}

fn to_value(v: impl serde::Serialize) -> Result<Value> {
    Ok(serde_json::to_value(v).map_err(anyhow::Error::from)?)
}

/// 回执里属于这笔操作的日志：上一笔操作的 `UserOperationEvent`（或 `BeforeExecution`）之后、本笔事件之前。
fn op_logs(logs: &[Log], entry_point: Address, hash: B256) -> Option<(usize, Vec<Log>)> {
    let is_ep = |l: &Log| l.address() == entry_point;
    let topic0 = |l: &Log| l.topics().first().copied();
    let pos = logs.iter().position(|l| {
        is_ep(l)
            && topic0(l) == Some(IEntryPoint::UserOperationEvent::SIGNATURE_HASH)
            && l.topics().get(1) == Some(&hash)
    })?;
    let start = logs[..pos]
        .iter()
        .rposition(|l| {
            is_ep(l)
                && (topic0(l) == Some(IEntryPoint::UserOperationEvent::SIGNATURE_HASH)
                    || topic0(l) == Some(IEntryPoint::BeforeExecution::SIGNATURE_HASH))
        })
        .map_or(0, |i| i + 1);
    Some((pos, logs[start..pos].to_vec()))
}

/// ERC-4337 `eth_getUserOperationReceipt` 格式。
fn user_op_receipt(
    receipt: &TransactionReceipt,
    entry_point: Address,
    hash: B256,
) -> Option<Value> {
    let logs = receipt.inner.logs();
    let (pos, own) = op_logs(logs, entry_point, hash)?;
    let event = IEntryPoint::UserOperationEvent::decode_log(&logs[pos].inner).ok()?;
    let reason = own
        .iter()
        .filter(|l| l.address() == entry_point)
        .find_map(|l| IEntryPoint::UserOperationRevertReason::decode_log(&l.inner).ok())
        .map(|r| r.data.revertReason.to_string())
        .unwrap_or_default();
    Some(json!({
        "userOpHash": hash,
        "entryPoint": entry_point,
        "sender": event.sender,
        "nonce": event.nonce,
        "paymaster": event.paymaster,
        "actualGasCost": event.actualGasCost,
        "actualGasUsed": event.actualGasUsed,
        "success": event.success,
        "reason": reason,
        "logs": own,
        "receipt": receipt,
    }))
}

impl Bundler {
    fn entry_point(&self) -> Address {
        self.sponsor.aa.entry_point
    }

    /// 自己代发不了、又配了第三方 bundler 时返回它。
    fn fallback_in_use(&self) -> Option<&Fallback> {
        self.fallback.as_ref().filter(|_| !self.relayer.healthy())
    }

    /// 处理一个请求，返回完整的 JSON-RPC 响应。
    pub async fn handle(&self, user_id: i64, req: Request) -> Value {
        let result = self.dispatch(user_id, &req.method, &req.params).await;
        jsonrpc::response(req.id, result)
    }

    async fn dispatch(
        &self,
        user_id: i64,
        method: &str,
        params: &Value,
    ) -> std::result::Result<Value, RpcError> {
        if !params.is_array() && !params.is_null() {
            return Err(RpcError::new(INVALID_REQUEST, "params must be an array"));
        }
        let r = match method {
            "eth_chainId" => Ok(json!(U64::from(self.sponsor.chain_id()))),
            "eth_supportedEntryPoints" => Ok(json!([self.entry_point()])),
            "pm_getPaymasterStubData" => self.stub(user_id, params).await,
            "pm_getPaymasterData" => self.paymaster_data(user_id, params).await,
            "eth_estimateUserOperationGas" => match self.fallback_in_use() {
                Some(f) => return f.request(method, params.clone()).await,
                None => self.estimate(user_id, params).await,
            },
            "pimlico_getUserOperationGasPrice" => match self.fallback_in_use() {
                Some(f) => return f.request(method, params.clone()).await,
                None => self.gas_price().await,
            },
            "eth_sendUserOperation" => self.send(user_id, params).await,
            "eth_getUserOperationReceipt" => self.receipt(user_id, params).await,
            "eth_getUserOperationByHash" => self.by_hash(user_id, params).await,
            _ => return Err(RpcError::new(METHOD_NOT_FOUND, "method not supported")),
        };
        r.map_err(RpcError::from)
    }

    async fn stub(&self, user_id: i64, params: &Value) -> Result<Value> {
        let op: UserOperation = param(params, 0, "userOperation")?;
        let ep: Address = param(params, 1, "entryPoint")?;
        let chain_id: U64 = param(params, 2, "chainId")?;
        to_value(self.sponsor.stub(user_id, &op, ep, chain_id).await?)
    }

    async fn paymaster_data(&self, user_id: i64, params: &Value) -> Result<Value> {
        let op: UserOperation = param(params, 0, "userOperation")?;
        let ep: Address = param(params, 1, "entryPoint")?;
        let chain_id: U64 = param(params, 2, "chainId")?;
        to_value(self.sponsor.sponsor(user_id, &op, ep, chain_id).await?)
    }

    async fn estimate(&self, user_id: i64, params: &Value) -> Result<Value> {
        let op: UserOperation = param(params, 0, "userOperation")?;
        let ep: Address = param(params, 1, "entryPoint")?;
        to_value(self.sponsor.estimate(user_id, &op, ep).await?)
    }

    async fn gas_price(&self) -> Result<Value> {
        let chain = &self.sponsor.chain;
        let gas_price = chain.gas_price().await?;
        let priority = chain.max_priority_fee().await?;
        Ok(sim::price_tiers(gas_price, priority))
    }

    async fn send(&self, user_id: i64, params: &Value) -> Result<Value> {
        let op: UserOperation = param(params, 0, "userOperation")?;
        let ep: Address = param(params, 1, "entryPoint")?;
        self.sponsor.check_entry_point(ep)?;
        if op.paymaster != Some(self.sponsor.aa.paymaster) {
            return Err(Reject::not_allowed("operation must use the platform paymaster").into());
        }
        if op.signature.is_empty() {
            return Err(Reject::invalid("missing account signature").into());
        }
        let hash = op.hash(ep, self.sponsor.chain_id())?;
        let row = db::by_hash(&self.sponsor.db, hash.as_slice())
            .await?
            .filter(|r| r.user_id == user_id)
            .ok_or_else(|| Reject::not_allowed("operation was not sponsored by this service"))?;
        match row.status {
            SponsorshipStatus::Signed => {}
            // 客户端重试同一笔：幂等返回。
            SponsorshipStatus::Submitted
            | SponsorshipStatus::Included
            | SponsorshipStatus::Failed => {
                return Ok(json!(hash));
            }
            _ => return Err(Reject::not_allowed("sponsorship expired; request a new one").into()),
        }
        let terms =
            SponsorTerms::decode(op.paymaster_data.as_ref().map_or(&[][..], |d| d.as_ref()))?;
        let op_json = to_value(&op)?;
        if let Some(f) = self.fallback_in_use() {
            f.request("eth_sendUserOperation", json!([op, ep]))
                .await
                .map_err(|e| anyhow::anyhow!("fallback bundler rejected: {}", e.message))?;
            db::mark_submitted(&self.sponsor.db, row.id, &op_json, true).await?;
            tracing::info!(%hash, "user operation forwarded to fallback bundler");
            return Ok(json!(hash));
        }
        if !db::mark_submitted(&self.sponsor.db, row.id, &op_json, false).await? {
            return Err(Reject::not_allowed("sponsorship expired; request a new one").into());
        }
        self.relayer
            .submit(Queued {
                user_op_hash: hash,
                op: op.pack()?,
                valid_until: terms.valid_until,
            })
            .await;
        Ok(json!(hash))
    }

    /// 本地记录：只给本人看；还没有链上结果、且经第三方提交的，转问第三方。
    async fn local(&self, user_id: i64, hash: B256) -> Result<Option<db::Sponsorship>> {
        Ok(db::by_hash(&self.sponsor.db, hash.as_slice())
            .await?
            .filter(|r| r.user_id == user_id))
    }

    async fn receipt(&self, user_id: i64, params: &Value) -> Result<Value> {
        let hash: B256 = param(params, 0, "userOpHash")?;
        let Some(row) = self.local(user_id, hash).await? else {
            return Ok(Value::Null);
        };
        let Some(tx) = row.tx_hash.as_deref().and_then(|t| B256::try_from(t).ok()) else {
            return self
                .ask_fallback(&row, "eth_getUserOperationReceipt", params)
                .await;
        };
        let receipt = self.sponsor.chain.receipt(tx).await?;
        Ok(receipt
            .and_then(|r| user_op_receipt(&r, self.entry_point(), hash))
            .unwrap_or(Value::Null))
    }

    async fn by_hash(&self, user_id: i64, params: &Value) -> Result<Value> {
        let hash: B256 = param(params, 0, "userOpHash")?;
        let Some(row) = self.local(user_id, hash).await? else {
            return Ok(Value::Null);
        };
        let Some(user_op) = row.user_op.clone() else {
            return Ok(Value::Null);
        };
        let tx = row.tx_hash.as_deref().and_then(|t| B256::try_from(t).ok());
        if tx.is_none() && row.via_fallback {
            return self
                .ask_fallback(&row, "eth_getUserOperationByHash", params)
                .await;
        }
        let block_hash = match tx {
            Some(t) => self
                .sponsor
                .chain
                .receipt(t)
                .await?
                .and_then(|r| r.block_hash),
            None => None,
        };
        Ok(json!({
            "userOperation": user_op,
            "entryPoint": self.entry_point(),
            "transactionHash": tx,
            "blockHash": block_hash,
            "blockNumber": row.block_number.map(|n| U64::from(n.unsigned_abs())),
        }))
    }

    async fn ask_fallback(
        &self,
        row: &db::Sponsorship,
        method: &str,
        params: &Value,
    ) -> Result<Value> {
        match self.fallback.as_ref().filter(|_| row.via_fallback) {
            Some(f) => f.request(method, params.clone()).await.or(Ok(Value::Null)),
            None => Ok(Value::Null),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use alloy::primitives::{Bytes, LogData, U256};

    use super::*;

    const EP: Address = Address::repeat_byte(0xee);

    fn log(address: Address, topics: Vec<B256>, data: Vec<u8>) -> Log {
        Log {
            inner: alloy::primitives::Log {
                address,
                data: LogData::new_unchecked(topics, Bytes::from(data)),
            },
            ..Default::default()
        }
    }

    fn uoe(hash: B256) -> Log {
        let e = IEntryPoint::UserOperationEvent {
            userOpHash: hash,
            sender: Address::repeat_byte(1),
            paymaster: Address::repeat_byte(2),
            nonce: U256::ZERO,
            success: true,
            actualGasCost: U256::from(10),
            actualGasUsed: U256::from(5),
        };
        let data = e.encode_log_data();
        log(EP, data.topics().to_vec(), data.data.to_vec())
    }

    #[test]
    fn op_logs_are_bounded_by_the_previous_event() {
        let (a, b) = (B256::repeat_byte(0xa), B256::repeat_byte(0xb));
        let token = Address::repeat_byte(0x77);
        let before = log(
            EP,
            vec![IEntryPoint::BeforeExecution::SIGNATURE_HASH],
            vec![],
        );
        let logs = vec![
            before,
            log(token, vec![B256::repeat_byte(1)], vec![]),
            uoe(a),
            log(token, vec![B256::repeat_byte(2)], vec![]),
            log(token, vec![B256::repeat_byte(3)], vec![]),
            uoe(b),
        ];
        let (pos, own) = op_logs(&logs, EP, a).unwrap();
        assert_eq!((pos, own.len()), (2, 1));
        let (pos, own) = op_logs(&logs, EP, b).unwrap();
        assert_eq!((pos, own.len()), (5, 2));
        assert_eq!(own[0].topics()[0], B256::repeat_byte(2));
        assert!(op_logs(&logs, EP, B256::ZERO).is_none());
    }
}
