//! 带主备切换的 RPC 客户端（plan 第 2 节「RPC 节点每类至少两家」），indexer、forum-api 的钱包地址校验与 forum-sponsor 共用。
//!
//! 每次调用从当前节点开始，失败就换下一个，直到都失败；切到备用节点后 `failback` 时间内不回切，
//! 之后先试主节点（备用节点通常更贵或限额更低）。
//!
//! 合约 revert 不是节点故障，不应触发切换：调用方在闭包里把 revert 转成 `Ok(Err(..))` 之类的业务结果。

use std::future::Future;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use alloy::providers::{DynProvider, Provider, ProviderBuilder};
use alloy::rpc::client::RpcClient;
use alloy::transports::http::reqwest;

use crate::ChainError;
use crate::config::{ChainConfig, Endpoint};

/// 单次 HTTP 请求超时；超时即视为节点故障并切换。
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

pub struct FailoverRpc {
    providers: Vec<(String, DynProvider)>,
    active: AtomicUsize,
    switched_at: Mutex<Option<Instant>>,
    failback: Duration,
}

/// 本机地址（开发、测试用的 anvil 与 bundler）不走 `HTTP_PROXY`。
pub fn is_loopback(url: &reqwest::Url) -> bool {
    matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
}

/// 访问 `url` 用的 HTTP 客户端：带超时，本机地址直连。
pub fn http_client(url: &reqwest::Url) -> Result<reqwest::Client, ChainError> {
    let b = reqwest::Client::builder().timeout(REQUEST_TIMEOUT);
    let b = if is_loopback(url) { b.no_proxy() } else { b };
    b.build().map_err(|e| ChainError::Config(e.to_string()))
}

/// 日志里只出现主机与端口，路径和查询串里的密钥不打印。
pub fn redact(url: &reqwest::Url) -> String {
    let host = url.host_str().unwrap_or("?");
    match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    }
}

impl FailoverRpc {
    /// 链配置里的全部节点，回切间隔取 `indexer.failback_secs`。
    pub fn from_config(cfg: &ChainConfig) -> Result<Self, ChainError> {
        Self::new(
            &cfg.endpoints(),
            Duration::from_secs(cfg.indexer.failback_secs),
        )
    }

    /// `endpoints` 按优先级排列（主节点在前），只用其中的 HTTP 地址。
    pub fn new(endpoints: &[&Endpoint], failback: Duration) -> Result<Self, ChainError> {
        let providers = endpoints
            .iter()
            .map(|e| {
                let url: reqwest::Url = e
                    .http
                    .parse()
                    .map_err(|err| ChainError::Config(format!("rpc url: {err}")))?;
                let client = RpcClient::new_http_with_client(http_client(&url)?, url.clone());
                let provider = ProviderBuilder::new()
                    .disable_recommended_fillers()
                    .connect_client(client)
                    .erased();
                Ok((redact(&url), provider))
            })
            .collect::<Result<Vec<_>, ChainError>>()?;
        if providers.is_empty() {
            return Err(ChainError::Config("no rpc endpoints".into()));
        }
        Ok(Self {
            providers,
            active: AtomicUsize::new(0),
            switched_at: Mutex::new(None),
            failback,
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

    /// 依次在各节点上执行 `f`，返回第一个成功结果；全部失败时返回最后一个错误。
    pub async fn call<T, F, Fut>(&self, what: &str, f: F) -> Result<T, ChainError>
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn redact_drops_path_and_query() {
        let url: reqwest::Url = "https://arb.example/v2/secret-key?x=1".parse().unwrap();
        assert_eq!(redact(&url), "https://arb.example");
        let url: reqwest::Url = "http://127.0.0.1:8545/".parse().unwrap();
        assert_eq!(redact(&url), "http://127.0.0.1:8545");
        assert!(is_loopback(&url));
    }
}
