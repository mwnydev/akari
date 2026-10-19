//! 按链配置建立 HTTP RPC 客户端（indexer 的主备数据源与 forum-api 的钱包地址校验共用）。

use std::time::Duration;

use alloy::providers::{DynProvider, Provider, ProviderBuilder};
use alloy::rpc::client::RpcClient;
use alloy::transports::http::reqwest;

use crate::ChainError;
use crate::config::ChainConfig;

/// 单次 HTTP 请求超时；超时即视为节点故障并切换。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// 按顺序（主节点在前）为每个节点建一个客户端，附带可以打进日志的名字（见 [`redact`]）。
pub fn http_providers(cfg: &ChainConfig) -> Result<Vec<(String, DynProvider)>, ChainError> {
    let client = |direct: bool| {
        let b = reqwest::Client::builder().timeout(REQUEST_TIMEOUT);
        // 本机节点（开发、测试用的 anvil）不走 HTTP_PROXY。
        let b = if direct { b.no_proxy() } else { b };
        b.build().map_err(|e| ChainError::Config(e.to_string()))
    };
    let (proxied, direct) = (client(false)?, client(true)?);
    cfg.endpoints()
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
        .collect()
}

/// 日志里只出现主机与端口，路径和查询串里的密钥不打印。
pub fn redact(url: &reqwest::Url) -> String {
    let host = url.host_str().unwrap_or("?");
    match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    }
}
