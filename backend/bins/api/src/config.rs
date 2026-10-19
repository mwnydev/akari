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
    /// 站长后台 Passkey 的 RP ID = 后台中心域名（ADR 0012：生产 `administrator.akari.cc`）。
    /// 与钱包一样绝不能是论坛主域名或其父域名，否则论坛页面就能请求这个 Passkey。
    pub admin_rp_id: String,
    /// 后台页面的 Origin（生产 `https://administrator.akari.cc`）。后台接口只接受这个来源，CORS 也只对它开放。
    pub admin_origin: String,
    /// 品牌名：Passkey 显示名称「品牌名 钱包签名（勿删）· 用户名」。
    pub brand_name: String,
    /// 论坛裁决 Passkey 的 RP ID（ADR 0012、0013）：论坛主机（生产 `akari.cc`，开发 `localhost`）。
    /// 只用于签署裁决，与钱包 RP ID 必须不同（两者都是 `localhost` 的本地开发除外）。
    pub ruling_rp_id: String,
    /// 裁决页面的 Origin（生产 `https://akari.cc`），必须是论坛来源之一、不能是钱包来源。
    pub ruling_origin: String,
    /// 仲裁 Safe（3 人 2/3，即合约的 `arbiter`）；只写进给签名人导入的 Safe 交易文件，未设置时文件里不带 Safe 地址。
    pub arbitration_safe: Option<Address>,
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
    pub const ADMIN_ORIGIN: &str = "https://administrator.akari.cc";
    /// 论坛裁决 Passkey 的 RP ID 与裁决页面就是论坛主站（ADR 0013）。
    pub const RULING_RP_ID: &str = FORUM_HOST;
    pub const RULING_ORIGIN: &str = FORUM_ORIGIN;
}

/// 页面来源与 Passkey 域名。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Sites {
    allowed_origins: Vec<String>,
    internal_hosts: Vec<String>,
    wallet_rp_id: String,
    wallet_origins: Vec<String>,
    admin_rp_id: String,
    admin_origin: String,
    ruling_rp_id: String,
    ruling_origin: String,
}

const SITE_VARS: [&str; 8] = [
    "ALLOWED_ORIGINS",
    "INTERNAL_HOSTS",
    "WALLET_RP_ID",
    "WALLET_ORIGINS",
    "ADMIN_RP_ID",
    "ADMIN_ORIGIN",
    "RULING_RP_ID",
    "RULING_ORIGIN",
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
    Ok(Sites {
        allowed_origins: list(&var("ALLOWED_ORIGINS")?),
        internal_hosts: list(&std::env::var("INTERNAL_HOSTS").unwrap_or_default()),
        wallet_rp_id: var("WALLET_RP_ID")?,
        wallet_origins: list(&var("WALLET_ORIGINS")?),
        admin_rp_id: var("ADMIN_RP_ID")?,
        admin_origin: var("ADMIN_ORIGIN")?,
        ruling_rp_id: var("RULING_RP_ID")?,
        ruling_origin: var("RULING_ORIGIN")?,
    })
}

fn production_sites() -> Sites {
    Sites {
        allowed_origins: vec![domains::FORUM_ORIGIN.to_owned()],
        internal_hosts: vec![domains::FORUM_HOST.to_owned()],
        wallet_rp_id: domains::WALLET_RP_ID.to_owned(),
        wallet_origins: vec![domains::WALLET_ORIGIN.to_owned()],
        admin_rp_id: domains::ADMIN_RP_ID.to_owned(),
        admin_origin: domains::ADMIN_ORIGIN.to_owned(),
        ruling_rp_id: domains::RULING_RP_ID.to_owned(),
        ruling_origin: domains::RULING_ORIGIN.to_owned(),
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
/// - 所有来源都是 https（开发环境可用本机 http）且互不重叠：钱包、后台来源都不是论坛来源，后台来源也不是钱包来源；
/// - 钱包与后台 Passkey 的 RP ID 必须覆盖自己的页面（钱包看第一个来源，其余为关联来源），两者不同，
///   且不能是论坛主站、其父域名或任何别的站点来源（论坛页面被 XSS 时也请求不到这些 Passkey）。
///   开发环境 RP ID 为 localhost 时各站只差端口，不检查隔离；
/// - 论坛裁决 Passkey 的 RP ID 就是裁决页面（某个论坛来源，不是钱包来源）的主机，且不等于钱包 RP ID：
///   论坛页面请求得到的凭证永远不是钱包凭证。
fn check_sites(sites: &Sites, deployment: Deployment) -> anyhow::Result<()> {
    ensure!(
        !sites.allowed_origins.is_empty(),
        "ALLOWED_ORIGINS must not be empty"
    );
    ensure!(
        !sites.wallet_origins.is_empty(),
        "WALLET_ORIGINS must not be empty"
    );
    let mut forum_hosts = vec![domains::FORUM_HOST.to_owned()];
    for o in &sites.allowed_origins {
        forum_hosts.push(origin_host(o, deployment)?);
    }
    let mut wallet_hosts = Vec::new();
    for o in &sites.wallet_origins {
        wallet_hosts.push(origin_host(o, deployment)?);
        ensure!(
            !sites.allowed_origins.contains(o),
            "wallet origin {o} must not also be a forum origin"
        );
    }
    let admin_host = origin_host(&sites.admin_origin, deployment)?;
    ensure!(
        !sites.allowed_origins.contains(&sites.admin_origin)
            && !sites.wallet_origins.contains(&sites.admin_origin),
        "ADMIN_ORIGIN must not be a forum or wallet origin"
    );
    let dev_loopback = |rp: &str| deployment == Deployment::Development && is_loopback(rp);
    ensure!(
        sites.admin_rp_id != sites.wallet_rp_id || dev_loopback(&sites.admin_rp_id),
        "ADMIN_RP_ID must differ from WALLET_RP_ID"
    );
    let admin_hosts = [admin_host];
    let realms = [
        (
            "WALLET_RP_ID",
            &sites.wallet_rp_id,
            &wallet_hosts[0],
            &admin_hosts[..],
        ),
        (
            "ADMIN_RP_ID",
            &sites.admin_rp_id,
            &admin_hosts[0],
            &wallet_hosts[..],
        ),
    ];
    for (name, rp, own_host, other_hosts) in realms {
        ensure!(!rp.is_empty(), "{name} must not be empty");
        ensure!(
            within(own_host, rp),
            "{name} {rp} does not cover its origin host {own_host}"
        );
        if dev_loopback(rp) {
            continue;
        }
        if let Some(h) = forum_hosts.iter().find(|h| within(h, rp)) {
            bail!("{name} {rp} must not be the forum host {h} or its parent domain");
        }
        if let Some(h) = other_hosts.iter().find(|h| within(h, rp)) {
            bail!("{name} {rp} must not cover the other site {h}");
        }
    }
    ensure!(
        sites.allowed_origins.contains(&sites.ruling_origin),
        "RULING_ORIGIN must be one of ALLOWED_ORIGINS"
    );
    ensure!(
        !sites.wallet_origins.contains(&sites.ruling_origin),
        "RULING_ORIGIN must not be a wallet origin"
    );
    ensure!(
        sites.ruling_rp_id == origin_host(&sites.ruling_origin, deployment)?,
        "RULING_RP_ID must be the host of RULING_ORIGIN"
    );
    ensure!(
        sites.ruling_rp_id != sites.wallet_rp_id || dev_loopback(&sites.ruling_rp_id),
        "RULING_RP_ID must differ from WALLET_RP_ID"
    );
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
        check_sites(&sites, deployment)?;
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
            admin_rp_id: sites.admin_rp_id,
            admin_origin: sites.admin_origin,
            brand_name: std::env::var("BRAND_NAME")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "Akari".to_owned()),
            ruling_rp_id: sites.ruling_rp_id,
            ruling_origin: sites.ruling_origin,
            arbitration_safe: opt_address("ARBITRATION_SAFE")?,
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

    use super::{Deployment, Sites, check_sites, cookie_secure, origin_host, production_sites};

    fn sites(forum: &[&str], rp: &str, wallet: &[&str]) -> Sites {
        Sites {
            allowed_origins: forum.iter().map(|&s| s.to_owned()).collect(),
            internal_hosts: Vec::new(),
            wallet_rp_id: rp.to_owned(),
            wallet_origins: wallet.iter().map(|&s| s.to_owned()).collect(),
            admin_rp_id: "admin.test".to_owned(),
            admin_origin: "https://admin.test".to_owned(),
            ruling_origin: forum.first().map_or_else(String::new, |&o| o.to_owned()),
            ruling_rp_id: forum
                .first()
                .and_then(|o| origin_host(o, Deployment::Development).ok())
                .unwrap_or_default(),
        }
    }

    fn check(s: &Sites, d: Deployment) -> Result<(), String> {
        check_sites(s, d).map_err(|e| e.to_string())
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
        assert_eq!(s.admin_rp_id, "administrator.akari.cc");
        assert_eq!(s.admin_origin, "https://administrator.akari.cc");
        check_sites(&s, Deployment::Production).unwrap();
        // 主站能请求的 RP ID 一律拒绝，钱包与后台都一样
        for rp in ["akari.cc", "cc"] {
            for admin in [false, true] {
                let mut bad = s.clone();
                if admin {
                    bad.admin_rp_id = rp.to_owned();
                } else {
                    bad.wallet_rp_id = rp.to_owned();
                }
                let err = check(&bad, Deployment::Production).unwrap_err();
                assert!(err.contains("must not be the forum host"), "{err}");
            }
        }
    }

    #[test]
    fn admin_passkey_is_isolated_from_forum_and_wallet() {
        let base = production_sites();
        let with = |f: &dyn Fn(&mut Sites)| {
            let mut s = base.clone();
            f(&mut s);
            check(&s, Deployment::Staging)
        };
        // 后台 RP ID 必须覆盖后台页面
        let err = with(&|s| s.admin_rp_id = "other.akari.cc".to_owned()).unwrap_err();
        assert!(err.contains("does not cover"), "{err}");
        // 与钱包共用 RP ID、或覆盖钱包页面
        assert!(with(&|s| s.admin_rp_id = s.wallet_rp_id.clone()).is_err());
        assert!(
            with(&|s| {
                s.admin_rp_id = "x.akari.cc".to_owned();
                s.admin_origin = "https://x.akari.cc".to_owned();
                s.wallet_origins = vec!["https://w.x.akari.cc".to_owned()];
                s.wallet_rp_id = "w.x.akari.cc".to_owned();
            })
            .unwrap_err()
            .contains("must not cover the other site")
        );
        // 后台页面不能同时是论坛或钱包页面
        assert!(with(&|s| s.admin_origin = "https://akari.cc".to_owned()).is_err());
        assert!(with(&|s| s.admin_origin = "https://wallet.akari.cc".to_owned()).is_err());
        assert!(with(&|s| s.admin_origin = "http://administrator.akari.cc".to_owned()).is_err());
        // 本地开发三站都在 localhost，只差端口
        let mut local = sites(
            &["http://localhost:3000"],
            "localhost",
            &["http://localhost:3001"],
        );
        local.admin_rp_id = "localhost".to_owned();
        local.admin_origin = "http://localhost:3002".to_owned();
        check(&local, Deployment::Development).unwrap();
        assert!(check(&local, Deployment::Staging).is_err());
    }

    #[test]
    fn passkey_rp_id_must_not_cover_the_forum() {
        let c = |forum: &str, rp: &str, d| {
            let wallet = format!("https://{rp}");
            check(&sites(&[forum], rp, &[&wallet]), d)
        };
        for d in [Deployment::Staging, Deployment::Development] {
            // 即使论坛来源配成别的域名，akari.cc 也不能作钱包 RP ID
            assert!(c("https://forum.test", "akari.cc", d).is_err());
            assert!(c("https://staging.example", "example", d).is_err());
            assert!(c("https://a.example", "a.example", d).is_err());
            c("https://a.example", "wallet.a.example", d).unwrap();
            // 后缀相同但不是子域名
            c("https://notexample", "example", d).unwrap();
            // RP ID 必须覆盖钱包页面
            let off = sites(
                &["https://a.example"],
                "wallet.a.example",
                &["https://w.test"],
            );
            assert!(check(&off, d).unwrap_err().contains("does not cover"));
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
    fn ruling_passkey_is_the_forum_host_and_never_the_wallet() {
        let base = production_sites();
        assert_eq!(base.ruling_rp_id, "akari.cc");
        assert_eq!(base.ruling_origin, "https://akari.cc");
        let with = |f: &dyn Fn(&mut Sites)| {
            let mut s = base.clone();
            f(&mut s);
            check(&s, Deployment::Staging)
        };
        with(&|_| ()).unwrap();
        // RP ID 必须等于裁决页面的主机
        assert!(with(&|s| s.ruling_rp_id = "cc".to_owned()).is_err());
        assert!(with(&|s| s.ruling_rp_id = "wallet.akari.cc".to_owned()).is_err());
        // 裁决页面必须是论坛来源，不能是钱包页面或别的站点
        assert!(with(&|s| s.ruling_origin = "https://evil.example".to_owned()).is_err());
        let err = with(&|s| {
            s.ruling_origin = "https://wallet.akari.cc".to_owned();
            s.ruling_rp_id = "wallet.akari.cc".to_owned();
            s.allowed_origins.push(s.ruling_origin.clone());
        })
        .unwrap_err();
        assert!(err.contains("wallet origin"), "{err}");
        // 本地开发论坛与钱包都是 localhost
        let local = sites(
            &["http://localhost:3000"],
            "localhost",
            &["http://localhost:3001"],
        );
        assert_eq!(local.ruling_rp_id, "localhost");
        check(&local, Deployment::Development).unwrap();
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
