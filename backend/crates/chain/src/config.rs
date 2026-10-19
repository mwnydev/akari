//! 链配置（plan 第 14 节「链配置文件」）：TOML 文件 + 环境变量覆盖 RPC 地址。
//!
//! RPC 地址里带服务商密钥，不进仓库也不进数据库：文件里可以留空，由
//! `CHAIN_RPC_PRIMARY_HTTP` / `CHAIN_RPC_PRIMARY_WS` / `CHAIN_RPC_BACKUP_HTTP` / `CHAIN_RPC_BACKUP_WS` 提供。
//! 示例见 `crates/chain/chain.example.toml`。

use std::time::Duration;

use alloy::primitives::{Address, U256};
use serde::Deserialize;

use crate::ChainError;
use crate::order::Market;

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
    /// 钱包初始化参数的各个版本（ADR 0008），按版本号升序、只增不改。
    #[serde(default)]
    pub wallet_init_versions: Vec<WalletInitVersion>,
    /// 账户抽象（ADR 0003、0004）：forum-sponsor 必填，其他程序不用。
    pub aa: Option<AaConfig>,
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
    /// 一次性担保 `Escrow`（也是统一保证金池）。
    pub escrow: Address,
    /// 分期担保 `StreamEscrow`。
    pub stream: Address,
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

impl Contracts {
    /// 地址是哪份担保合约；EscrowConfig 与其他地址为 `None`。
    pub fn market(&self, contract: Address) -> Option<Market> {
        if contract == self.escrow {
            Some(Market::Escrow)
        } else if contract == self.stream {
            Some(Market::Stream)
        } else {
            None
        }
    }
}

/// 一个版本的钱包初始化参数（ADR 0008）：Kernel v3.3 账户的 CREATE2 地址由它们和 Passkey 公钥共同决定，
/// 所以一经使用就不能修改；换 Paymaster 或代币时新增一个版本。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WalletInitVersion {
    pub version: u32,
    pub kernel_implementation: Address,
    pub kernel_factory: Address,
    /// 部署账户（首笔操作的 initCode）经它调用 `kernel_factory`。
    pub kernel_meta_factory: Address,
    pub webauthn_validator: Address,
    pub entry_point: Address,
    /// 初始化时授权的代付合约（ADR 0004：代付费在验证阶段扣，第一笔操作前就要有授权）。
    pub paymaster: Address,
    pub approve_token: Address,
    /// 授权额度（十进制或 0x 十六进制字符串；通常为 uint256 最大值）。
    pub approve_amount: U256,
}

/// EntryPoint v0.7 与自有 Paymaster。`factories` 是首笔操作 `initCode` 允许使用的工厂
/// （Kernel v3.3 meta factory，ADR 0003）：钱包只能按平台的初始化参数部署。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AaConfig {
    pub entry_point: Address,
    pub paymaster: Address,
    pub factories: Vec<Address>,
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
        vec![
            self.contracts.escrow,
            self.contracts.stream,
            self.contracts.config,
        ]
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
        let c = &self.contracts;
        if c.escrow.is_zero()
            || c.stream.is_zero()
            || c.config.is_zero()
            || self.token.address.is_zero()
        {
            return bad("contract and token addresses must be non-zero");
        }
        if c.escrow == c.stream || c.escrow == c.config || c.stream == c.config {
            return bad("escrow, stream and config must be different contracts");
        }
        let mut last = 0;
        for v in &self.wallet_init_versions {
            if v.version <= last {
                return bad(
                    "wallet_init_versions must be listed by strictly increasing version >= 1",
                );
            }
            last = v.version;
            let addrs = [
                v.kernel_implementation,
                v.kernel_factory,
                v.kernel_meta_factory,
                v.webauthn_validator,
                v.entry_point,
                v.paymaster,
                v.approve_token,
            ];
            if addrs.iter().any(|a| a.is_zero()) {
                return bad("wallet init version addresses must be non-zero");
            }
        }
        if let Some(aa) = &self.aa
            && (aa.entry_point.is_zero()
                || aa.paymaster.is_zero()
                || aa.factories.is_empty()
                || aa.factories.iter().any(|f| f.is_zero()))
        {
            return bad("aa addresses must be non-zero and factories non-empty");
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
        assert_eq!(cfg.indexed_contracts().len(), 3);
        let c = &cfg.contracts;
        assert_eq!(c.market(c.stream), Some(Market::Stream));
        assert_eq!(c.market(c.config), None);
        let [v1] = cfg.wallet_init_versions.as_slice() else {
            panic!("one wallet init version");
        };
        assert_eq!((v1.version, v1.approve_amount), (1, U256::MAX));
        assert_eq!(v1.approve_token, cfg.token.address);
        let aa = cfg.aa.as_ref().unwrap();
        assert_eq!(aa.factories.len(), 1);
    }

    #[test]
    fn wallet_init_versions_must_increase() {
        // 示例里的版本表在文件末尾：再接一份同号的版本。
        let v1 = EXAMPLE.find("[[wallet_init_versions]]").unwrap();
        let err = ChainConfig::parse(&format!("{EXAMPLE}\n{}", &EXAMPLE[v1..]), env).unwrap_err();
        assert!(err.to_string().contains("increasing"), "{err}");
        let decimal = EXAMPLE.replace(
            "approve_amount = \"0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff\"",
            "approve_amount = \"1000000\"",
        );
        let cfg = ChainConfig::parse(&decimal, env).unwrap();
        assert_eq!(
            cfg.wallet_init_versions[0].approve_amount,
            U256::from(1_000_000)
        );
    }

    #[test]
    fn zero_aa_addresses_are_rejected() {
        let text = EXAMPLE.replace(
            "factories = [\"0xd703aaE79538628d27099B8c4f621bE4CCd142d5\"]",
            "factories = []",
        );
        let err = ChainConfig::parse(&text, env).unwrap_err();
        assert!(err.to_string().contains("factories"), "{err}");
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
