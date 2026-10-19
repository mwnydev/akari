//! 配置全部来自环境变量（生产由 systemd-creds 注入密钥）。

use anyhow::{Context, bail};
use forum_chain::config::WalletInitVersion;
use forum_core::Usdt;
use forum_core::eip712::Address;
use forum_core::trade::{MAX_BASE_GAS, MAX_FEE_BPS, MAX_MIN_FEE, TradeLimits};

#[derive(Debug, Clone)]
pub struct Config {
    pub listen: String,
    pub database_url: String,
    pub valkey_url: String,
    /// 64 字节十六进制：会话与设备 Cookie 的签名密钥。
    pub cookie_secret: Vec<u8>,
    /// ≥ 32 字节十六进制：验证码 HMAC 密钥。
    pub auth_secret: Vec<u8>,
    /// 写操作允许的 Origin，逗号分隔，如 `https://forum.example,https://wallet.example`。
    pub allowed_origins: Vec<String>,
    /// 生产必须为 true；本地 http 开发可设 `COOKIE_SECURE=false`。
    pub cookie_secure: bool,
    /// 为空时使用不校验的 NoopCaptcha（仅开发环境）。
    pub turnstile_secret: Option<String>,
    /// 本站主机名，逗号分隔（如 `forum.example,www.forum.example`）；判断帖子里的站外链接用。
    pub internal_hosts: Vec<String>,
    /// 钱包 Passkey 的 RP ID = 钱包中心域名（plan 第 5 节），绝不是论坛主域名。
    pub wallet_rp_id: String,
    /// 钱包页面的 Origin，逗号分隔；第一个必须在 `WALLET_RP_ID` 之下，其余为关联来源。
    /// 钱包接口只接受这些来源，这些来源也自动加入写操作白名单。
    pub wallet_origins: Vec<String>,
    /// 品牌名：Passkey 显示名称「品牌名 钱包签名（勿删）· 用户名」。
    pub brand_name: String,
    pub chain: ChainConfig,
    /// `CHAIN_CONFIG` 指向的链配置文件（RPC 用于校验钱包地址）；未设置时钱包地址登记不可用。
    pub chain_file: Option<forum_chain::config::ChainConfig>,
    pub trade_limits: TradeLimits,
    /// 本机 forum-sponsor，如 `http://127.0.0.1:8090`（`SPONSOR_URL`）；与 `SPONSOR_API_TOKEN` 同时设置才启用。
    pub sponsor_url: Option<String>,
    pub sponsor_token: Option<String>,
}

/// 担保合约所在链的公开参数。
#[derive(Debug, Clone)]
pub struct ChainConfig {
    pub chain_id: u64,
    /// 未配置时报价接口不可用（开发环境可只测论坛与 Passkey）。
    pub escrow: Option<Address>,
    pub usdt: Option<Address>,
    /// 钱包初始化参数版本（ADR 0008）；为空时不能登记钱包地址。
    pub wallet_init_versions: Vec<WalletInitVersion>,
}

impl ChainConfig {
    pub fn wallet_init_version(&self, version: u32) -> Option<&WalletInitVersion> {
        self.wallet_init_versions
            .iter()
            .find(|v| v.version == version)
    }
}

/// 链参数：设置了 `CHAIN_CONFIG` 时全部取自链配置文件（与 indexer 同一份），
/// 否则用 `CHAIN_ID` / `ESCROW_ADDRESS` / `USDT_ADDRESS`（只用于不接链的本地开发）。
fn chain() -> anyhow::Result<(ChainConfig, Option<forum_chain::config::ChainConfig>)> {
    let Some(path) = std::env::var_os("CHAIN_CONFIG").filter(|p| !p.is_empty()) else {
        let chain = ChainConfig {
            // 默认 Arbitrum Sepolia；主网 42161。
            chain_id: num_or("CHAIN_ID", 421_614)?,
            escrow: opt_address("ESCROW_ADDRESS")?,
            usdt: opt_address("USDT_ADDRESS")?,
            wallet_init_versions: Vec::new(),
        };
        return Ok((chain, None));
    };
    for k in ["CHAIN_ID", "ESCROW_ADDRESS", "USDT_ADDRESS"] {
        if std::env::var(k).is_ok_and(|v| !v.is_empty()) {
            bail!("{k} conflicts with CHAIN_CONFIG (the chain config file is the single source)");
        }
    }
    let file = forum_chain::config::ChainConfig::load(path.as_ref())?;
    let address = |a: alloy::primitives::Address| Address(a.into_array());
    let chain = ChainConfig {
        chain_id: file.chain_id,
        escrow: Some(address(file.contracts.escrow)),
        usdt: Some(address(file.token.address)),
        wallet_init_versions: file.wallet_init_versions.clone(),
    };
    Ok((chain, Some(file)))
}

fn list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}

fn var(name: &str) -> anyhow::Result<String> {
    std::env::var(name).with_context(|| format!("missing env {name}"))
}

fn hex_var(name: &str, min_len: usize) -> anyhow::Result<Vec<u8>> {
    let bytes = hex::decode(var(name)?).with_context(|| format!("{name} must be hex"))?;
    if bytes.len() < min_len {
        bail!("{name} must be at least {min_len} bytes");
    }
    Ok(bytes)
}

fn opt_address(name: &str) -> anyhow::Result<Option<Address>> {
    match std::env::var(name) {
        Ok(v) if !v.is_empty() => {
            Ok(Some(v.parse().map_err(|_| {
                anyhow::anyhow!("{name} must be a 0x address")
            })?))
        }
        _ => Ok(None),
    }
}

fn num_or<T: std::str::FromStr>(name: &str, default: T) -> anyhow::Result<T> {
    match std::env::var(name) {
        Ok(v) if !v.is_empty() => v
            .parse()
            .map_err(|_| anyhow::anyhow!("{name} must be a number")),
        _ => Ok(default),
    }
}

/// 链上当前参数（USDT 最小单位）；默认值即 plan 第 9 节初始值。站长经金库 Safe 调参后同步修改这里。
fn trade_limits() -> anyhow::Result<TradeLimits> {
    let d = TradeLimits::default();
    let l = TradeLimits {
        min_order: Usdt(num_or("TRADE_MIN_ORDER", d.min_order.0)?),
        order_cap: Usdt(num_or("TRADE_ORDER_CAP", d.order_cap.0)?),
        fee_bps: num_or("TRADE_FEE_BPS", d.fee_bps)?,
        min_fee: Usdt(num_or("TRADE_MIN_FEE", d.min_fee.0)?),
        base_gas: Usdt(num_or("TRADE_BASE_GAS", d.base_gas.0)?),
    };
    if l.min_order > l.order_cap
        || l.fee_bps > MAX_FEE_BPS
        || l.min_fee > MAX_MIN_FEE
        || l.base_gas > MAX_BASE_GAS
    {
        bail!("TRADE_* limits outside the contract's hard bounds");
    }
    Ok(l)
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let (chain, chain_file) = chain()?;
        Ok(Self {
            listen: std::env::var("API_LISTEN").unwrap_or_else(|_| "127.0.0.1:8080".to_owned()),
            database_url: var("DATABASE_URL")?,
            valkey_url: var("VALKEY_URL")?,
            cookie_secret: hex_var("COOKIE_SECRET", 64)?,
            auth_secret: hex_var("AUTH_SECRET", 32)?,
            allowed_origins: list(&var("ALLOWED_ORIGINS")?),
            cookie_secure: std::env::var("COOKIE_SECURE").map_or(true, |v| v != "false"),
            turnstile_secret: std::env::var("TURNSTILE_SECRET")
                .ok()
                .filter(|s| !s.is_empty()),
            internal_hosts: list(&std::env::var("INTERNAL_HOSTS").unwrap_or_default()),
            wallet_rp_id: var("WALLET_RP_ID")?,
            wallet_origins: {
                let o = list(&var("WALLET_ORIGINS")?);
                if o.is_empty() {
                    bail!("WALLET_ORIGINS must not be empty");
                }
                o
            },
            brand_name: std::env::var("BRAND_NAME")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "Akari".to_owned()),
            chain,
            chain_file,
            trade_limits: trade_limits()?,
            sponsor_url: std::env::var("SPONSOR_URL").ok().filter(|s| !s.is_empty()),
            sponsor_token: std::env::var("SPONSOR_API_TOKEN")
                .ok()
                .filter(|s| !s.is_empty()),
        })
    }
}
