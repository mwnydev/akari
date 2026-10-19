//! 带主备切换的 RPC 数据源（plan 第 2 节「RPC 节点每类至少两家」）。
//!
//! 每次调用从当前节点开始，失败就换下一个，直到都失败；切到备用节点后 `failback` 时间内不回切，
//! 之后先试主节点（备用节点通常更贵或限额更低）。

use std::future::Future;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use alloy::eips::BlockId;
use alloy::primitives::{Address, B256, TxKind};
use alloy::providers::{DynProvider, Provider, ProviderBuilder};
use alloy::rpc::client::RpcClient;
use alloy::rpc::types::{BlockNumberOrTag, Filter, TransactionInput, TransactionRequest};
use alloy::transports::http::reqwest;
use forum_chain::ChainError;
use forum_chain::config::ChainConfig;
use forum_chain::event::{BlockRef, RawLog};
use forum_chain::reorg::{BlockTag, ChainSource};
use forum_chain::snapshot::{self, OrderSnapshot};

/// 单次 HTTP 请求超时；超时即视为节点故障并切换。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

pub struct RpcSource {
    providers: Vec<(String, DynProvider)>,
    contracts: Vec<Address>,
    active: AtomicUsize,
    switched_at: Mutex<Option<Instant>>,
    failback: Duration,
}

impl RpcSource {
    pub fn new(cfg: &ChainConfig) -> Result<Self, ChainError> {
        let client = |direct: bool| {
            let b = reqwest::Client::builder().timeout(REQUEST_TIMEOUT);
            // 本机节点（开发、测试用的 anvil）不走 HTTP_PROXY。
            let b = if direct { b.no_proxy() } else { b };
            b.build().map_err(|e| ChainError::Config(e.to_string()))
        };
        let (proxied, direct) = (client(false)?, client(true)?);
        let providers = cfg
            .endpoints()
            .into_iter()
            .map(|e| {
                let url: reqwest::Url = e
                    .http
                    .parse()
                    .map_err(|err| ChainError::Config(format!("rpc url: {err}")))?;
                let loopback = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
                let http = if loopback { &direct } else { &proxied };
                let client = RpcClient::new_http_with_client(http.clone(), url.clone());
                let provider = ProviderBuilder::new()
                    .disable_recommended_fillers()
                    .connect_client(client)
                    .erased();
                Ok((redact(&url), provider))
            })
            .collect::<Result<_, ChainError>>()?;
        Ok(Self {
            providers,
            contracts: cfg.indexed_contracts(),
            active: AtomicUsize::new(0),
            switched_at: Mutex::new(None),
            failback: Duration::from_secs(cfg.indexer.failback_secs),
        })
    }

    /// 当前节点下标（测试与日志用）。
    pub fn active(&self) -> usize {
        self.active.load(Ordering::Relaxed)
    }

    fn start_index(&self) -> usize {
        let active = self.active();
        if active == 0 {
            return 0;
        }
        let Ok(mut at) = self.switched_at.lock() else {
            return active;
        };
        if at.is_some_and(|t| t.elapsed() >= self.failback) {
            *at = None;
            self.active.store(0, Ordering::Relaxed);
            tracing::info!("rpc failback to primary");
            return 0;
        }
        active
    }

    async fn call<T, F, Fut>(&self, what: &str, f: F) -> Result<T, ChainError>
    where
        F: Fn(DynProvider) -> Fut,
        Fut: Future<Output = Result<T, ChainError>>,
    {
        let start = self.start_index();
        let n = self.providers.len();
        let mut last = ChainError::Rpc("no rpc endpoints".into());
        for i in (0..n).map(|k| (start + k) % n) {
            let (name, provider) = &self.providers[i];
            match f(provider.clone()).await {
                Ok(v) => {
                    if i != self.active() {
                        tracing::warn!(endpoint = %name, "rpc switched endpoint");
                        self.active.store(i, Ordering::Relaxed);
                        if let Ok(mut at) = self.switched_at.lock() {
                            *at = Some(Instant::now());
                        }
                    }
                    return Ok(v);
                }
                Err(e) => {
                    tracing::warn!(endpoint = %name, call = what, error = %e, "rpc call failed");
                    last = e;
                }
            }
        }
        Err(last)
    }
}

/// 日志里只出现主机与端口，路径和查询串里的密钥不打印。
pub(crate) fn redact(url: &reqwest::Url) -> String {
    let host = url.host_str().unwrap_or("?");
    match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
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
        self.call("eth_getBlockByNumber", |p| async move {
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
        self.call("eth_getLogs", |p| {
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
        contract: Address,
        order_id: u64,
        at: B256,
    ) -> Result<Option<OrderSnapshot>, ChainError> {
        let input = snapshot::encode_call(order_id)?;
        self.call("eth_call getOrder", |p| {
            let tx = TransactionRequest {
                to: Some(TxKind::Call(contract)),
                input: TransactionInput::new(input.clone()),
                ..Default::default()
            };
            async move {
                let out = p.call(tx).block(BlockId::hash(at)).await?;
                snapshot::decode_return(&out)
            }
        })
        .await
    }
}
