//! 链配置（plan 第 14 节「链配置文件」）：TOML 文件 + 环境变量覆盖 RPC 地址。
//!
//! RPC 地址里带服务商密钥，不进仓库也不进数据库：文件里可以留空，由
//! `CHAIN_RPC_PRIMARY_HTTP` / `CHAIN_RPC_PRIMARY_WS` / `CHAIN_RPC_BACKUP_HTTP` / `CHAIN_RPC_BACKUP_WS` 提供。
//! 示例见 `crates/chain/chain.example.toml`。

use std::time::Duration;

use alloy::primitives::Address;
use serde::Deserialize;

use crate::ChainError;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChainConfig {
    pub chain_id: u64,
    pub name: String,
    #[serde(default)]
    pub rpc: RpcConfig,
    pub contracts: Contracts,
    pub token: Token,
    #[serde(default)]
    pub confirmation: ConfirmationPolicy,
    #[serde(default)]
    pub indexer: IndexerPolicy,
}

/// 两家 RPC 服务商（plan 第 2 节「每类至少两家」）。`ws` 只用于订阅即时事件，缺省时只靠定时补拉。
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RpcConfig {
    #[serde(default)]
    pub primary: Endpoint,
    pub backup: Option<Endpoint>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    #[serde(default)]
    pub http: String,
    pub ws: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contracts {
    pub escrow: Address,
    pub config: Address,
    /// 合约部署所在区块；首次启动从这里开始补拉。
    pub start_block: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Token {
    pub symbol: String,
    pub address: Address,
    pub decimals: u8,
}

/// 链上状态分级确认（plan 第 2、8 节）：latest 立即写入；safe / finalized 优先用 RPC 的同名区块标记，
/// 节点不支持标记时退回「最新块往前 N 块」。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ConfirmationPolicy {
    pub use_block_tags: bool,
    pub fallback_safe_depth: u64,
    pub fallback_finalized_depth: u64,
}

impl Default for ConfirmationPolicy {
    fn default() -> Self {
        // Arbitrum 约 0.25 秒一块：240 块 ≈ 1 分钟，4800 块 ≈ 20 分钟（与 L1 最终确认的延迟同量级）。
        Self {
            use_block_tags: true,
            fallback_safe_depth: 240,
            fallback_finalized_depth: 4800,
        }
    }
}

/// 省调用（plan 第 2 节）：WebSocket 订阅即时显示，每 `reconcile_interval_ms` 按区块范围批量补拉校对。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct IndexerPolicy {
    pub reconcile_interval_ms: u64,
    /// 一次 `eth_getLogs` 最多跨多少块（服务商普遍限制在 2000–10000）。
    pub max_block_range: u64,
    /// 每隔几轮校对查询一次 safe / finalized 标记（它们变化慢，不必每轮都查）。
    pub promote_every: u32,
    /// 切到备用节点后多久再试主节点。
    pub failback_secs: u64,
}

impl Default for IndexerPolicy {
    fn default() -> Self {
        Self {
            reconcile_interval_ms: 5000,
            max_block_range: 2000,
            promote_every: 6,
            failback_secs: 300,
        }
    }
}

impl IndexerPolicy {
    pub fn reconcile_interval(&self) -> Duration {
        Duration::from_millis(self.reconcile_interval_ms)
    }
}

impl ChainConfig {
    /// 读取文件并用进程环境变量覆盖 RPC 地址。
    pub fn load(path: &std::path::Path) -> Result<Self, ChainError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| ChainError::Config(format!("{}: {e}", path.display())))?;
        Self::parse(&text, |k| std::env::var(k).ok())
    }

    /// 解析 TOML，`env` 提供覆盖值（测试里传入闭包，不读进程环境）。
    pub fn parse(text: &str, env: impl Fn(&str) -> Option<String>) -> Result<Self, ChainError> {
        let mut cfg: Self = toml::from_str(text).map_err(|e| ChainError::Config(e.to_string()))?;
        let set = |slot: &mut String, key: &str| {
            if let Some(v) = env(key).filter(|v| !v.is_empty()) {
                *slot = v;
            }
        };
        let set_opt = |slot: &mut Option<String>, key: &str| {
            if let Some(v) = env(key).filter(|v| !v.is_empty()) {
                *slot = Some(v);
            }
        };
        set(&mut cfg.rpc.primary.http, "CHAIN_RPC_PRIMARY_HTTP");
        set_opt(&mut cfg.rpc.primary.ws, "CHAIN_RPC_PRIMARY_WS");
        if let Some(http) = env("CHAIN_RPC_BACKUP_HTTP").filter(|v| !v.is_empty()) {
            cfg.rpc.backup.get_or_insert_with(Endpoint::default).http = http;
        }
        if let Some(backup) = cfg.rpc.backup.as_mut() {
            set_opt(&mut backup.ws, "CHAIN_RPC_BACKUP_WS");
        }
        cfg.validate()?;
        Ok(cfg)
    }

    /// 按顺序排列的节点：主节点在前。
    pub fn endpoints(&self) -> Vec<&Endpoint> {
        std::iter::once(&self.rpc.primary)
            .chain(self.rpc.backup.as_ref())
            .collect()
    }

    /// indexer 订阅与补拉日志的合约。
    pub fn indexed_contracts(&self) -> Vec<Address> {
        vec![self.contracts.escrow, self.contracts.config]
    }

    fn validate(&self) -> Result<(), ChainError> {
        let bad = |m: &str| Err(ChainError::Config(m.to_owned()));
        if self.chain_id == 0 {
            return bad("chain_id must be set");
        }
        for e in self.endpoints() {
            if !(e.http.starts_with("http://") || e.http.starts_with("https://")) {
                return bad("rpc http url missing or not http(s) (set CHAIN_RPC_*_HTTP)");
            }
            if e.ws
                .as_deref()
                .is_some_and(|w| !(w.starts_with("ws://") || w.starts_with("wss://")))
            {
                return bad("rpc ws url must be ws(s)");
            }
        }
        if self.contracts.escrow.is_zero()
            || self.contracts.config.is_zero()
            || self.token.address.is_zero()
        {
            return bad("contract and token addresses must be non-zero");
        }
        if self.token.decimals > 36 {
            return bad("token decimals out of range");
        }
        if self.indexer.max_block_range == 0
            || self.indexer.reconcile_interval_ms == 0
            || self.indexer.promote_every == 0
        {
            return bad("indexer intervals and ranges must be positive");
        }
        if self.confirmation.fallback_safe_depth > self.confirmation.fallback_finalized_depth {
            return bad("fallback_safe_depth must not exceed fallback_finalized_depth");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    const EXAMPLE: &str = include_str!("../chain.example.toml");

    fn env(k: &str) -> Option<String> {
        match k {
            "CHAIN_RPC_PRIMARY_HTTP" => Some("https://primary.example/key".to_owned()),
            "CHAIN_RPC_PRIMARY_WS" => Some("wss://primary.example/key".to_owned()),
            "CHAIN_RPC_BACKUP_HTTP" => Some("https://backup.example/key".to_owned()),
            _ => None,
        }
    }

    #[test]
    fn example_parses_with_env_rpc() {
        let cfg = ChainConfig::parse(EXAMPLE, env).unwrap();
        assert_eq!(cfg.chain_id, 42_161);
        assert_eq!(cfg.token.decimals, 6);
        assert_eq!(
            cfg.rpc.primary.ws.as_deref(),
            Some("wss://primary.example/key")
        );
        let backup = cfg.rpc.backup.as_ref().unwrap();
        assert_eq!(backup.http, "https://backup.example/key");
        assert_eq!(backup.ws, None);
        assert_eq!(cfg.endpoints().len(), 2);
        assert_eq!(cfg.indexer, IndexerPolicy::default());
    }

    #[test]
    fn missing_rpc_is_rejected() {
        let err = ChainConfig::parse(EXAMPLE, |_| None).unwrap_err();
        assert!(err.to_string().contains("CHAIN_RPC"), "{err}");
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let text = format!("surprise = 1\n{EXAMPLE}");
        let err = ChainConfig::parse(&text, env).unwrap_err();
        assert!(err.to_string().contains("surprise"), "{err}");
    }
}
