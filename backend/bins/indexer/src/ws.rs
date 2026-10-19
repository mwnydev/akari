//! WebSocket 订阅合约日志，用于即时显示（plan 第 2 节「indexer 省调用」）。
//!
//! 订阅只是加速：断线、漏推、推送了随后被回滚的日志都不影响正确性，定时校对会补齐和纠正。
//! 断线后按 1 秒起翻倍（最长 30 秒）退避，轮流尝试主备节点。

use std::time::{Duration, Instant};

use alloy::primitives::Address;
use alloy::providers::{Provider, ProviderBuilder, WsConnect};
use alloy::rpc::types::Filter;
use alloy::transports::http::reqwest::Url;
use forum_chain::event::RawLog;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const MAX_BACKOFF: Duration = Duration::from_secs(30);
/// 连接保持这么久才算稳定，之后断线从最短退避重新开始。
const STABLE_AFTER: Duration = Duration::from_secs(60);

/// 一直运行到 `shutdown`。`urls` 为空时立即返回。
pub async fn run(
    urls: Vec<String>,
    contracts: Vec<Address>,
    out: mpsc::Sender<RawLog>,
    shutdown: CancellationToken,
) {
    if urls.is_empty() {
        tracing::info!("no websocket endpoint configured, relying on periodic reconcile");
        return;
    }
    let filter = Filter::new().address(contracts);
    let mut backoff = Duration::from_secs(1);
    for url in urls.iter().cycle() {
        let name = url
            .parse::<Url>()
            .map_or_else(|_| "?".to_owned(), |u| forum_chain::rpc::redact(&u));
        let started = Instant::now();
        tokio::select! {
            () = shutdown.cancelled() => return,
            r = stream(url, &filter, &out) => match r {
                Ok(()) => tracing::warn!(endpoint = %name, "websocket subscription ended"),
                Err(e) => tracing::warn!(endpoint = %name, error = %e, "websocket subscription failed"),
            },
        }
        if out.is_closed() {
            return;
        }
        if started.elapsed() >= STABLE_AFTER {
            backoff = Duration::from_secs(1);
        }
        tokio::select! {
            () = shutdown.cancelled() => return,
            () = tokio::time::sleep(backoff) => {}
        }
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

/// 连接并转发日志，直到连接断开。
async fn stream(
    url: &str,
    filter: &Filter,
    out: &mpsc::Sender<RawLog>,
) -> Result<(), alloy::transports::TransportError> {
    let provider = ProviderBuilder::new()
        .disable_recommended_fillers()
        .connect_ws(WsConnect::new(url))
        .await?;
    let mut sub = provider.subscribe_logs(filter).await?;
    tracing::info!("websocket log subscription active");
    loop {
        let log = match sub.recv().await {
            Ok(log) => log,
            // 处理不及丢了推送：定时校对会补上。
            Err(RecvError::Lagged(n)) => {
                tracing::warn!(skipped = n, "websocket subscription lagged");
                continue;
            }
            Err(RecvError::Closed) => return Ok(()),
        };
        match RawLog::try_from(log) {
            Ok(raw) => {
                if out.send(raw).await.is_err() {
                    return Ok(());
                }
            }
            Err(e) => tracing::warn!(error = %e, "ignoring malformed websocket log"),
        }
    }
}
