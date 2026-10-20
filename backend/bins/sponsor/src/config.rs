//! 配置：链配置文件（`CHAIN_CONFIG`，含 `[aa]`）+ 环境变量。私钥不在这里，见 [`crate::keys`]。

use std::net::SocketAddr;
use std::time::Duration;

use alloy::primitives::{Address, U256};
use anyhow::{Context, bail};
use forum_chain::config::{AaConfig, ChainConfig};

use crate::policy::Contracts;

/// 单笔代付费硬上限 1 USDT，与 `FixedFeePaymaster.MAX_FEE`、钱包确认页一致（plan 第 9 节，不可在线改）。
pub const MAX_SPONSOR_FEE: u128 = 1_000_000;
/// 默认代付费 0.05 USDT（plan 第 9 节）。
pub const DEFAULT_SPONSOR_FEE: u128 = 50_000;
/// 代付签名有效期 2 分钟（plan 第 5 节）。
pub const VALIDITY_SECS: u64 = 120;
const ETH: u128 = 1_000_000_000_000_000_000;

#[derive(Debug, Clone)]
pub struct Config {
    /// 只监听本机：forum-api 经 127.0.0.1 转发（`SPONSOR_LISTEN`，默认 127.0.0.1:8090）。
    pub listen: SocketAddr,
    /// forum-api 调用本服务时带的共享令牌（`SPONSOR_API_TOKEN`，≥ 32 字符）。
    pub api_token: String,
    pub database_url: String,
    pub chain: ChainConfig,
    pub aa: AaConfig,
    pub policy: Policy,
    pub relayer: RelayerConfig,
    pub alerts: AlertConfig,
    /// 私有代发器不健康时转交的第三方 bundler（`FALLBACK_BUNDLER_URL`，带服务商密钥，不打印）。
    pub fallback_bundler: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Policy {
    /// 付款前操作的代付费（`SPONSOR_FEE`，USDT 最小单位）。
    pub sponsor_fee: u128,
    /// 单笔操作最多让 Paymaster 押金承担多少 wei（gas 上限之和 × maxFeePerGas，`SPONSOR_MAX_OP_COST_WEI`）。
    pub max_op_cost: U256,
}

#[derive(Debug, Clone)]
pub struct RelayerConfig {
    /// 微批窗口（`SPONSOR_BATCH_WINDOW_MS`，100–200，plan 第 5 节）。
    pub batch_window: Duration,
    pub max_batch: usize,
    /// 多久没上链就提价重发。
    pub resend_after: Duration,
    pub max_bumps: u32,
}

#[derive(Debug, Clone)]
pub struct AlertConfig {
    /// Paymaster 在 EntryPoint 的押金告警线（`SPONSOR_ALERT_DEPOSIT_WEI`，默认 0.05 ETH）。
    pub deposit_min: U256,
    /// 代发热钱包余额告警线（`SPONSOR_ALERT_HOT_WALLET_WEI`，默认 0.01 ETH）；低于它也视为代发器不健康。
    pub hot_wallet_min: U256,
    pub interval: Duration,
    /// 同一种告警多久内只发一次。
    pub dedupe: Duration,
}

fn var(name: &str) -> anyhow::Result<String> {
    std::env::var(name).with_context(|| format!("missing env {name}"))
}

fn opt(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn parse_or<T: std::str::FromStr>(name: &str, default: T) -> anyhow::Result<T> {
    match opt(name) {
        Some(v) => v
            .parse()
            .map_err(|_| anyhow::anyhow!("{name} has an invalid value")),
        None => Ok(default),
    }
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let chain = ChainConfig::load(var("CHAIN_CONFIG")?.as_ref())?;
        let aa = chain
            .aa
            .clone()
            .context("chain config has no [aa] section (entry_point, paymaster, factories)")?;
        let api_token = var("SPONSOR_API_TOKEN")?;
        if api_token.len() < 32 {
            bail!("SPONSOR_API_TOKEN must be at least 32 characters");
        }
        let policy = Policy {
            sponsor_fee: parse_or("SPONSOR_FEE", DEFAULT_SPONSOR_FEE)?,
            max_op_cost: parse_or("SPONSOR_MAX_OP_COST_WEI", U256::from(5 * ETH / 1000))?,
        };
        if policy.sponsor_fee > MAX_SPONSOR_FEE {
            bail!("SPONSOR_FEE exceeds the 1 USDT hard cap");
        }
        let window_ms: u64 = parse_or("SPONSOR_BATCH_WINDOW_MS", 150)?;
        if !(100..=200).contains(&window_ms) {
            bail!("SPONSOR_BATCH_WINDOW_MS must be within 100..=200");
        }
        Ok(Self {
            listen: parse_or("SPONSOR_LISTEN", SocketAddr::from(([127, 0, 0, 1], 8090)))?,
            api_token,
            database_url: var("DATABASE_URL")?,
            chain,
            aa,
            policy,
            relayer: RelayerConfig {
                batch_window: Duration::from_millis(window_ms),
                ..RelayerConfig::default()
            },
            alerts: AlertConfig {
                deposit_min: parse_or("SPONSOR_ALERT_DEPOSIT_WEI", U256::from(5 * ETH / 100))?,
                hot_wallet_min: parse_or("SPONSOR_ALERT_HOT_WALLET_WEI", U256::from(ETH / 100))?,
                ..AlertConfig::default()
            },
            fallback_bundler: opt("FALLBACK_BUNDLER_URL"),
        })
    }

    pub fn contracts(&self) -> Contracts {
        Contracts {
            token: self.chain.token.address,
            escrow: self.chain.contracts.escrow,
            stream: self.chain.contracts.stream,
        }
    }

    pub fn entry_point(&self) -> Address {
        self.aa.entry_point
    }

    /// 拍卖授权与成交证明的 EIP-712 签名域（本链 + Escrow）。
    pub fn domain(&self) -> forum_core::eip712::Domain {
        forum_core::eip712::Domain {
            chain_id: self.chain.chain_id,
            verifying_contract: forum_core::eip712::Address(
                self.chain.contracts.escrow.into_array(),
            ),
        }
    }
}

impl Default for RelayerConfig {
    fn default() -> Self {
        Self {
            batch_window: Duration::from_millis(150),
            max_batch: 8,
            resend_after: Duration::from_secs(10),
            max_bumps: 3,
        }
    }
}

impl Default for AlertConfig {
    fn default() -> Self {
        Self {
            deposit_min: U256::from(5 * ETH / 100),
            hot_wallet_min: U256::from(ETH / 100),
            interval: Duration::from_secs(60),
            dedupe: Duration::from_secs(6 * 3600),
        }
    }
}
