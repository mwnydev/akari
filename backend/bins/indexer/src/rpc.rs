//! indexer 的链数据源：主备切换由 [`FailoverRpc`] 负责（plan 第 2 节「RPC 节点每类至少两家」）。

use alloy::eips::BlockId;
use alloy::primitives::{Address, B256, TxKind};
use alloy::providers::Provider;
use alloy::rpc::types::{BlockNumberOrTag, Filter, TransactionInput, TransactionRequest};
use forum_chain::ChainError;
use forum_chain::config::ChainConfig;
use forum_chain::event::{BlockRef, RawLog};
use forum_chain::order::Market;
use forum_chain::reorg::{BlockTag, ChainSource};
use forum_chain::rpc::FailoverRpc;
use forum_chain::snapshot::{self, OrderSnapshot};

pub struct RpcSource {
    rpc: FailoverRpc,
    contracts: Vec<Address>,
}

impl RpcSource {
    pub fn new(cfg: &ChainConfig) -> Result<Self, ChainError> {
        Ok(Self {
            rpc: FailoverRpc::from_config(cfg)?,
            contracts: cfg.indexed_contracts(),
        })
    }

    /// 当前节点下标（测试与日志用）。
    pub fn active(&self) -> usize {
        self.rpc.active()
    }
}

impl ChainSource for RpcSource {
    async fn block(&self, tag: BlockTag) -> Result<Option<BlockRef>, ChainError> {
        let tag = match tag {
            BlockTag::Latest => BlockNumberOrTag::Latest,
            BlockTag::Safe => BlockNumberOrTag::Safe,
            BlockTag::Finalized => BlockNumberOrTag::Finalized,
            BlockTag::Number(n) => BlockNumberOrTag::Number(n),
        };
        self.rpc
            .call("eth_getBlockByNumber", |p| async move {
                let block = p.get_block_by_number(tag).await?;
                Ok(block.map(|b| BlockRef {
                    number: b.header.number,
                    hash: b.header.hash,
                }))
            })
            .await
    }

    async fn logs(&self, from: u64, to: u64) -> Result<Vec<RawLog>, ChainError> {
        let filter = Filter::new()
            .address(self.contracts.clone())
            .from_block(from)
            .to_block(to);
        self.rpc
            .call("eth_getLogs", |p| {
                let filter = filter.clone();
                async move {
                    p.get_logs(&filter)
                        .await?
                        .into_iter()
                        .map(RawLog::try_from)
                        .collect()
                }
            })
            .await
    }

    async fn order_snapshot(
        &self,
        market: Market,
        contract: Address,
        order_id: u64,
        at: B256,
    ) -> Result<Option<OrderSnapshot>, ChainError> {
        let input = snapshot::encode_call(market, order_id)?;
        self.rpc
            .call("eth_call getOrder", |p| {
                let tx = TransactionRequest {
                    to: Some(TxKind::Call(contract)),
                    input: TransactionInput::new(input.clone()),
                    ..Default::default()
                };
                async move {
                    let out = p.call(tx).block(BlockId::hash(at)).await?;
                    snapshot::decode_return(market, &out)
                }
            })
            .await
    }

    async fn block_time(&self, hash: B256) -> Result<Option<u64>, ChainError> {
        self.rpc
            .call("eth_getBlockByHash", |p| async move {
                let block = p.get_block_by_hash(hash).await?;
                Ok(block.map(|b| b.header.timestamp))
            })
            .await
    }
}
