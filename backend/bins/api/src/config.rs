//! 配置全部来自环境变量（生产由 systemd-creds 注入密钥）。

use anyhow::{Context, bail, ensure};
use forum_auth::passkey::webauthn::Url;
use forum_chain::config::WalletInitVersion;
use forum_core::Usdt;
use forum_core::eip712::Address;
use forum_core::trade::{MAX_BASE_GAS, MAX_FEE_BPS, MAX_MIN_FEE, TradeLimits};

#[derive(Debug, Clone)]
pub struct Config {
    pub listen: String,
    pub database_url: String,
    pub valkey_url: String,
    /// Valkey 键与推送频道前缀（`KV_PREFIX`，见 [`forum_kv::namespace_from_env`]）。
    pub kv_namespace: String,
    /// 64 字节十六进制：会话与设备 Cookie 的签名密钥。
    pub cookie_secret: Vec<u8>,
    /// ≥ 32 字节十六进制：验证码 HMAC 密钥。
    pub auth_secret: Vec<u8>,
    /// 论坛页面的 Origin（写操作白名单），逗号分隔；钱包页面来源另见 `wallet_origins`。
    pub allowed_origins: Vec<String>,
    /// 只有 `AKARI_ENV=development` 才能设 `COOKIE_SECURE=false`（本地 http 开发）。
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

/// 部署环境（`AKARI_ENV`）。只认这三个值，拼错直接拒绝启动；未设置视为 `development`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Deployment {
    /// 域名固定为 [`domains`] 里的正式域名，不读 `ALLOWED_ORIGINS` 等环境变量（ADR 0012）。
    Production,
    /// 域名取自环境变量，必须全部是 https。
    Staging,
    /// 同 `Staging`，但允许本机（localhost / 127.0.0.1 / [::1]）的 http 来源与 `COOKIE_SECURE=false`。
    Development,
}

impl Deployment {
    fn parse(value: Option<&str>) -> anyhow::Result<Self> {
        match value.unwrap_or_default() {
            "production" => Ok(Self::Production),
            "staging" => Ok(Self::Staging),
            "" | "development" => Ok(Self::Development),
            other => bail!("AKARI_ENV must be production, staging or development, got {other:?}"),
        }
    }
}

/// ADR 0012 正式域名。只经代码审查与发布流程修改，不进环境变量，也不进后台在线配置。
pub mod domains {
    /// 论坛主站；也是可注册域名，会话 Cookie 在这里保持 host-only。
    pub const FORUM_HOST: &str = "akari.cc";
    pub const FORUM_ORIGIN: &str = "https://akari.cc";
    /// 钱包 Passkey 的 RP ID 与钱包页面。
    pub const WALLET_RP_ID: &str = "wallet.akari.cc";
    pub const WALLET_ORIGIN: &str = "https://wallet.akari.cc";
    /// 后台 Passkey 的 RP ID（ADR 0013，仅站长）。
    pub const ADMIN_RP_ID: &str = "administrator.akari.cc";
}

/// 页面来源与 Passkey 域名。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Sites {
    allowed_origins: Vec<String>,
    internal_hosts: Vec<String>,
    wallet_rp_id: String,
    wallet_origins: Vec<String>,
}

const SITE_VARS: [&str; 4] = [
    "ALLOWED_ORIGINS",
    "INTERNAL_HOSTS",
    "WALLET_RP_ID",
    "WALLET_ORIGINS",
];

fn sites(deployment: Deployment) -> anyhow::Result<Sites> {
    if deployment == Deployment::Production {
        for k in SITE_VARS {
            if std::env::var_os(k).is_some_and(|v| !v.is_empty()) {
                bail!(
                    "{k} must not be set in production: the domains are fixed in code (ADR 0012)"
                );
            }
        }
        return Ok(production_sites());
    }
    let wallet_origins = list(&var("WALLET_ORIGINS")?);
    ensure!(
        !wallet_origins.is_empty(),
        "WALLET_ORIGINS must not be empty"
    );
    Ok(Sites {
        allowed_origins: list(&var("ALLOWED_ORIGINS")?),
        internal_hosts: list(&std::env::var("INTERNAL_HOSTS").unwrap_or_default()),
        wallet_rp_id: var("WALLET_RP_ID")?,
        wallet_origins,
    })
}

fn production_sites() -> Sites {
    Sites {
        allowed_origins: vec![domains::FORUM_ORIGIN.to_owned()],
        internal_hosts: vec![domains::FORUM_HOST.to_owned()],
        wallet_rp_id: domains::WALLET_RP_ID.to_owned(),
        wallet_origins: vec![domains::WALLET_ORIGIN.to_owned()],
    }
}

fn is_loopback(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}

/// `host` 等于 `domain` 或在它之下。
fn within(host: &str, domain: &str) -> bool {
    host == domain
        || host
            .strip_suffix(domain)
            .is_some_and(|rest| rest.ends_with('.'))
}

/// 来源必须是 `scheme://host[:port]` 原样（不带路径、末尾斜杠或大写，否则与浏览器发来的 `Origin` 永远对不上），
/// 且是 https；只有开发环境允许本机 http。返回主机名。
fn origin_host(origin: &str, deployment: Deployment) -> anyhow::Result<String> {
    let url = Url::parse(origin).with_context(|| format!("origin {origin:?} is not a URL"))?;
    ensure!(
        url.origin().ascii_serialization() == origin,
        "origin {origin:?} must be exactly scheme://host[:port]"
    );
    let host = url.host_str().unwrap_or_default().to_owned();
    let local_http =
        deployment == Deployment::Development && url.scheme() == "http" && is_loopback(&host);
    ensure!(
        url.scheme() == "https" || local_http,
        "origin {origin} must be https"
    );
    Ok(host)
}

/// 启动检查（ADR 0012）：
/// - 所有来源都是 https（开发环境可用本机 http）；钱包来源不在论坛来源里；
/// - 钱包与后台 Passkey 的 RP ID 不能是论坛主站或其父域名，否则论坛页面（含被 XSS 时）能直接请求这些 Passkey。
///   开发环境 RP ID 为 localhost 时论坛与钱包只差端口，不检查这一项。
fn check_sites(
    sites: &Sites,
    passkey_rp_ids: &[&str],
    deployment: Deployment,
) -> anyhow::Result<()> {
    ensure!(
        !sites.allowed_origins.is_empty(),
        "ALLOWED_ORIGINS must not be empty"
    );
    let mut forum_hosts = vec![domains::FORUM_HOST.to_owned()];
    for o in &sites.allowed_origins {
        forum_hosts.push(origin_host(o, deployment)?);
    }
    for o in &sites.wallet_origins {
        origin_host(o, deployment)?;
        ensure!(
            !sites.allowed_origins.contains(o),
            "wallet origin {o} must not also be a forum origin"
        );
    }
    for rp in passkey_rp_ids {
        ensure!(!rp.is_empty(), "passkey RP ID must not be empty");
        if deployment == Deployment::Development && is_loopback(rp) {
            continue;
        }
        if let Some(h) = forum_hosts.iter().find(|h| within(h, rp)) {
            bail!("passkey RP ID {rp} must not be the forum host {h} or its parent domain");
        }
    }
    Ok(())
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
    /// 本站用到的全部合约：担保、分期、参数、代币、EntryPoint、Paymaster、Kernel 实现 / 工厂 / 验证器。
    /// 不能登记为提币地址；链上读代理只放行对它们（与调用者自己的钱包）的 `eth_call`。
    pub contracts: Vec<Address>,
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
        let escrow = opt_address("ESCROW_ADDRESS")?;
        let usdt = opt_address("USDT_ADDRESS")?;
        let chain = ChainConfig {
            // 默认 Arbitrum Sepolia；主网 42161。
            chain_id: num_or("CHAIN_ID", 421_614)?,
            escrow,
            usdt,
            wallet_init_versions: Vec::new(),
            contracts: escrow.into_iter().chain(usdt).collect(),
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
        contracts: known_contracts(&file).into_iter().map(address).collect(),
    };
    Ok((chain, Some(file)))
}

fn known_contracts(file: &forum_chain::config::ChainConfig) -> Vec<alloy::primitives::Address> {
    let c = &file.contracts;
    let mut all = vec![c.escrow, c.stream, c.config, file.token.address];
    if let Some(aa) = &file.aa {
        all.extend([aa.entry_point, aa.paymaster]);
        all.extend(&aa.factories);
    }
    for v in &file.wallet_init_versions {
        all.extend([
            v.kernel_implementation,
            v.kernel_factory,
            v.kernel_meta_factory,
            v.webauthn_validator,
            v.entry_point,
            v.paymaster,
            v.approve_token,
        ]);
    }
    all.sort();
    all.dedup();
    all
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

fn cookie_secure(value: Option<&str>, deployment: Deployment) -> anyhow::Result<bool> {
    match value.unwrap_or_default() {
        "" | "true" => Ok(true),
        "false" if deployment == Deployment::Development => Ok(false),
        "false" => bail!("COOKIE_SECURE=false is only allowed with AKARI_ENV=development"),
        other => bail!("COOKIE_SECURE must be true or false, got {other:?}"),
    }
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let deployment = Deployment::parse(std::env::var("AKARI_ENV").ok().as_deref())?;
        let sites = sites(deployment)?;
        let mut passkey_rp_ids = vec![sites.wallet_rp_id.as_str()];
        if deployment == Deployment::Production {
            passkey_rp_ids.push(domains::ADMIN_RP_ID);
        }
        check_sites(&sites, &passkey_rp_ids, deployment)?;
        let (chain, chain_file) = chain()?;
        Ok(Self {
            listen: std::env::var("API_LISTEN").unwrap_or_else(|_| "127.0.0.1:8080".to_owned()),
            database_url: var("DATABASE_URL")?,
            valkey_url: var("VALKEY_URL")?,
            kv_namespace: forum_kv::namespace_from_env()?,
            cookie_secret: hex_var("COOKIE_SECRET", 64)?,
            auth_secret: hex_var("AUTH_SECRET", 32)?,
            allowed_origins: sites.allowed_origins,
            cookie_secure: cookie_secure(
                std::env::var("COOKIE_SECURE").ok().as_deref(),
                deployment,
            )?,
            turnstile_secret: std::env::var("TURNSTILE_SECRET")
                .ok()
                .filter(|s| !s.is_empty()),
            internal_hosts: sites.internal_hosts,
            wallet_rp_id: sites.wallet_rp_id,
            wallet_origins: sites.wallet_origins,
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::{Deployment, Sites, check_sites, cookie_secure, domains, production_sites};

    fn sites(forum: &[&str], rp: &str, wallet: &[&str]) -> Sites {
        Sites {
            allowed_origins: forum.iter().map(|&s| s.to_owned()).collect(),
            internal_hosts: Vec::new(),
            wallet_rp_id: rp.to_owned(),
            wallet_origins: wallet.iter().map(|&s| s.to_owned()).collect(),
        }
    }

    fn check(s: &Sites, d: Deployment) -> Result<(), String> {
        check_sites(s, &[&s.wallet_rp_id], d).map_err(|e| e.to_string())
    }

    #[test]
    fn deployment_is_parsed_strictly() {
        assert_eq!(
            Deployment::parse(Some("production")).unwrap(),
            Deployment::Production
        );
        assert_eq!(
            Deployment::parse(Some("staging")).unwrap(),
            Deployment::Staging
        );
        assert_eq!(Deployment::parse(None).unwrap(), Deployment::Development);
        for typo in ["prod", "Production", "production ", "dev"] {
            assert!(Deployment::parse(Some(typo)).is_err(), "{typo}");
        }
    }

    #[test]
    fn production_uses_the_adr_0012_domains() {
        let s = production_sites();
        assert_eq!(s.allowed_origins, ["https://akari.cc"]);
        assert_eq!(s.wallet_origins, ["https://wallet.akari.cc"]);
        assert_eq!(s.wallet_rp_id, "wallet.akari.cc");
        check_sites(
            &s,
            &[domains::WALLET_RP_ID, domains::ADMIN_RP_ID],
            Deployment::Production,
        )
        .unwrap();
        // 主站能请求的 RP ID 一律拒绝，包括后台
        for rp in ["akari.cc", "cc"] {
            let err = check_sites(&s, &[domains::WALLET_RP_ID, rp], Deployment::Production)
                .unwrap_err()
                .to_string();
            assert!(err.contains("must not be the forum host"), "{err}");
        }
    }

    #[test]
    fn passkey_rp_id_must_not_cover_the_forum() {
        let w = &["https://wallet.akari.cc"];
        for d in [Deployment::Staging, Deployment::Development] {
            // 即使论坛来源配成别的域名，akari.cc 也不能作钱包 RP ID
            assert!(check(&sites(&["https://forum.test"], "akari.cc", w), d).is_err());
            assert!(check(&sites(&["https://staging.example"], "example", w), d).is_err());
            assert!(check(&sites(&["https://a.example"], "a.example", w), d).is_err());
            check(&sites(&["https://a.example"], "wallet.a.example", w), d).unwrap();
            // 后缀相同但不是子域名
            check(&sites(&["https://notexample"], "example", w), d).unwrap();
        }
    }

    #[test]
    fn origins_must_be_https_and_exact() {
        let ok = sites(
            &["https://akari.cc"],
            "wallet.akari.cc",
            &["https://wallet.akari.cc"],
        );
        check(&ok, Deployment::Staging).unwrap();
        for (forum, wallet) in [
            ("http://akari.cc", "https://wallet.akari.cc"),
            ("https://akari.cc", "http://wallet.akari.cc"),
            ("https://akari.cc/", "https://wallet.akari.cc"),
            ("https://Akari.cc", "https://wallet.akari.cc"),
            ("https://akari.cc:443", "https://wallet.akari.cc"),
            ("akari.cc", "https://wallet.akari.cc"),
            ("https://akari.cc", "https://wallet.akari.cc/pay"),
        ] {
            for d in [Deployment::Staging, Deployment::Development] {
                let s = sites(&[forum], "wallet.akari.cc", &[wallet]);
                assert!(check(&s, d).is_err(), "{forum} {wallet} {d:?}");
            }
        }
        assert!(
            check(
                &sites(&[], "wallet.akari.cc", &["https://wallet.akari.cc"]),
                Deployment::Staging
            )
            .is_err()
        );
        // 钱包来源不能同时是论坛来源
        let both = sites(
            &["https://akari.cc", "https://wallet.akari.cc"],
            "wallet.akari.cc",
            &["https://wallet.akari.cc"],
        );
        assert!(check(&both, Deployment::Staging).is_err());
    }

    #[test]
    fn local_http_only_in_development() {
        let local = sites(
            &["http://localhost:3000", "http://127.0.0.1:3000"],
            "localhost",
            &["http://localhost:3001"],
        );
        check(&local, Deployment::Development).unwrap();
        assert!(check(&local, Deployment::Staging).is_err());
        let lan = sites(
            &["http://192.168.1.5:3000"],
            "localhost",
            &["http://localhost:3001"],
        );
        assert!(check(&lan, Deployment::Development).is_err());
    }

    #[test]
    fn insecure_cookies_only_in_development() {
        assert!(cookie_secure(None, Deployment::Production).unwrap());
        assert!(cookie_secure(Some("true"), Deployment::Staging).unwrap());
        assert!(!cookie_secure(Some("false"), Deployment::Development).unwrap());
        assert!(cookie_secure(Some("false"), Deployment::Production).is_err());
        assert!(cookie_secure(Some("false"), Deployment::Staging).is_err());
        assert!(cookie_secure(Some("flase"), Deployment::Development).is_err());
    }

    #[test]
    fn known_contracts_cover_markets_token_aa_and_kernel() {
        let text = include_str!("../../../crates/chain/chain.example.toml");
        let file = forum_chain::config::ChainConfig::parse(text, |k| {
            (k == "CHAIN_RPC_PRIMARY_HTTP").then(|| "https://rpc.example".to_owned())
        })
        .unwrap();
        let all = super::known_contracts(&file);
        let v = &file.wallet_init_versions[0];
        for a in [
            file.contracts.escrow,
            file.contracts.stream,
            file.contracts.config,
            file.token.address,
            v.kernel_factory,
            v.kernel_meta_factory,
            v.webauthn_validator,
            v.entry_point,
            v.paymaster,
        ] {
            assert!(all.contains(&a), "{a}");
        }
        // 示例里 token、EntryPoint、Paymaster 与初始化参数里的重复，只出现一次。
        assert_eq!(all.iter().filter(|&&a| a == file.token.address).count(), 1);
    }
}
