//! 链上读写（主备切换见 [`FailoverRpc`]）。
//!
//! 节点返回的 JSON-RPC 错误（合约 revert、nonce 太低……）说明节点本身正常，不切换节点，
//! 以 [`NodeError`] 交给调用方处理；只有连接失败、超时才算节点故障。

use alloy::eips::BlockId;
use alloy::primitives::{Address, B256, Bytes, TxKind, U256};
use alloy::providers::Provider;
use alloy::rpc::types::simulate::{SimBlock, SimCallResult, SimulatePayload};
use alloy::rpc::types::{Filter, Log, TransactionInput, TransactionReceipt, TransactionRequest};
use alloy::sol_types::SolCall;
use alloy::transports::{RpcError, TransportError};
use forum_chain::ChainError;
use forum_chain::aa::IEntryPoint;
use forum_chain::order::Market;
use forum_chain::rpc::FailoverRpc;
use forum_chain::snapshot::{self, Terms};

/// 节点明确拒绝或执行回滚。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeError {
    pub code: i64,
    pub message: String,
    /// revert 数据（有的话）。
    pub data: Option<Bytes>,
}

impl std::fmt::Display for NodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.code)?;
        if let Some(d) = &self.data {
            write!(f, " data={d}")?;
        }
        Ok(())
    }
}

pub type NodeResult<T> = Result<T, NodeError>;

fn split<T>(r: Result<T, TransportError>) -> Result<NodeResult<T>, ChainError> {
    match r {
        Ok(v) => Ok(Ok(v)),
        Err(RpcError::ErrorResp(p)) => Ok(Err(NodeError {
            code: p.code,
            message: p.message.to_string(),
            data: p.as_revert_data(),
        })),
        Err(e) => Err(e.into()),
    }
}

pub fn call_request(from: Address, to: Address, data: Bytes) -> TransactionRequest {
    TransactionRequest {
        from: Some(from),
        to: Some(TxKind::Call(to)),
        input: TransactionInput::new(data),
        ..Default::default()
    }
}

/// Arbitrum NodeInterface（虚拟合约）：估算一笔交易的 L1 数据费折合多少 L2 gas。
const NODE_INTERFACE: Address =
    alloy::primitives::address!("0x00000000000000000000000000000000000000C8");
const ARBITRUM_CHAINS: [u64; 3] = [42_161, 421_614, 42_170];

alloy::sol! {
    interface INodeInterface {
        function gasEstimateL1Component(address to, bool contractCreation, bytes calldata data)
            external payable returns (uint64 gasEstimateForL1, uint256 baseFee, uint256 l1BaseFeeEstimate);
    }
}

pub struct Chain {
    pub rpc: FailoverRpc,
    pub chain_id: u64,
    pub entry_point: Address,
}

impl Chain {
    pub async fn code_len(&self, addr: Address) -> Result<usize, ChainError> {
        self.rpc
            .call("eth_getCode", |p| async move {
                Ok(p.get_code_at(addr).await?.len())
            })
            .await
    }

    pub async fn balance(&self, addr: Address) -> Result<U256, ChainError> {
        self.rpc
            .call("eth_getBalance", |p| async move {
                Ok(p.get_balance(addr).await?)
            })
            .await
    }

    pub async fn gas_price(&self) -> Result<u128, ChainError> {
        self.rpc
            .call(
                "eth_gasPrice",
                |p| async move { Ok(p.get_gas_price().await?) },
            )
            .await
    }

    pub async fn max_priority_fee(&self) -> Result<u128, ChainError> {
        self.rpc
            .call("eth_maxPriorityFeePerGas", |p| async move {
                Ok(p.get_max_priority_fee_per_gas().await?)
            })
            .await
    }

    pub async fn block_number(&self) -> Result<u64, ChainError> {
        self.rpc
            .call("eth_blockNumber", |p| async move {
                Ok(p.get_block_number().await?)
            })
            .await
    }

    /// 待上链交易也算进去的 nonce。
    pub async fn pending_nonce(&self, addr: Address) -> Result<u64, ChainError> {
        self.rpc
            .call("eth_getTransactionCount", |p| async move {
                Ok(p.get_transaction_count(addr).pending().await?)
            })
            .await
    }

    pub async fn call(&self, tx: &TransactionRequest) -> Result<NodeResult<Bytes>, ChainError> {
        self.rpc
            .call("eth_call", |p| {
                let tx = tx.clone();
                async move { split(p.call(tx).block(BlockId::latest()).await) }
            })
            .await
    }

    pub async fn estimate_gas(
        &self,
        tx: &TransactionRequest,
    ) -> Result<NodeResult<u64>, ChainError> {
        self.rpc
            .call("eth_estimateGas", |p| {
                let tx = tx.clone();
                async move { split(p.estimate_gas(tx).await) }
            })
            .await
    }

    /// 在 latest 状态上按顺序执行 `calls`（后一笔看得到前一笔的状态变化），不检查发送方余额与 nonce。
    pub async fn simulate(
        &self,
        calls: Vec<TransactionRequest>,
    ) -> Result<NodeResult<Vec<SimCallResult>>, ChainError> {
        let payload = SimulatePayload {
            block_state_calls: vec![SimBlock {
                block_overrides: None,
                state_overrides: None,
                calls,
            }],
            trace_transfers: false,
            validation: false,
            return_full_transactions: false,
        };
        self.rpc
            .call("eth_simulateV1", |p| {
                let payload = payload.clone();
                async move {
                    let r = split(p.simulate(&payload).latest().await)?;
                    Ok(r.map(|blocks| blocks.into_iter().flat_map(|b| b.calls).collect()))
                }
            })
            .await
    }

    pub async fn send_raw(&self, raw: &[u8]) -> Result<NodeResult<B256>, ChainError> {
        self.rpc
            .call("eth_sendRawTransaction", |p| async move {
                split(
                    p.send_raw_transaction(raw)
                        .await
                        .map(|pending| *pending.tx_hash()),
                )
            })
            .await
    }

    pub async fn receipt(&self, hash: B256) -> Result<Option<TransactionReceipt>, ChainError> {
        self.rpc
            .call("eth_getTransactionReceipt", |p| async move {
                Ok(p.get_transaction_receipt(hash).await?)
            })
            .await
    }

    pub async fn logs(&self, filter: &Filter) -> Result<Vec<Log>, ChainError> {
        self.rpc
            .call("eth_getLogs", |p| {
                let filter = filter.clone();
                async move { Ok(p.get_logs(&filter).await?) }
            })
            .await
    }

    /// Paymaster 在 EntryPoint 里的 ETH 押金。
    pub async fn deposit_of(&self, account: Address) -> Result<U256, ChainError> {
        let data = IEntryPoint::balanceOfCall { account }.abi_encode();
        let tx = call_request(Address::ZERO, self.entry_point, data.into());
        let out = self
            .call(&tx)
            .await?
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        IEntryPoint::balanceOfCall::abi_decode_returns(&out)
            .map_err(|e| ChainError::Decode(e.to_string()))
    }

    /// Escrow 订单是否已标记发货（latest 状态，plan 第 5 节「直接认链上 latest 状态」）。不存在的订单返回 `None`。
    pub async fn escrow_delivered(
        &self,
        escrow: Address,
        order_id: u64,
    ) -> Result<Option<bool>, ChainError> {
        let data = snapshot::encode_call(Market::Escrow, order_id)?;
        let tx = call_request(Address::ZERO, escrow, data);
        let out = self
            .call(&tx)
            .await?
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        Ok(snapshot::decode_return(Market::Escrow, &out)?.map(
            |o| matches!(o.terms, Terms::Escrow { confirm_deadline, .. } if confirm_deadline != 0),
        ))
    }

    /// 这笔 `handleOps` 的 L1 数据费折合的 L2 gas；不是 Arbitrum 时为 0。
    pub async fn l1_gas(&self, handle_ops: Bytes) -> Result<u64, ChainError> {
        if !ARBITRUM_CHAINS.contains(&self.chain_id) {
            return Ok(0);
        }
        let data = INodeInterface::gasEstimateL1ComponentCall {
            to: self.entry_point,
            contractCreation: false,
            data: handle_ops,
        }
        .abi_encode();
        let tx = call_request(Address::ZERO, NODE_INTERFACE, data.into());
        let out = self
            .call(&tx)
            .await?
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        INodeInterface::gasEstimateL1ComponentCall::abi_decode_returns(&out)
            .map(|r| r.gasEstimateForL1)
            .map_err(|e| ChainError::Decode(e.to_string()))
    }
}
