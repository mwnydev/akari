//! 配置全部来自环境变量（生产由 systemd-creds 注入密钥）。

use anyhow::{Context, bail};

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

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            listen: std::env::var("API_LISTEN").unwrap_or_else(|_| "127.0.0.1:8080".to_owned()),
            database_url: var("DATABASE_URL")?,
            valkey_url: var("VALKEY_URL")?,
            cookie_secret: hex_var("COOKIE_SECRET", 64)?,
            auth_secret: hex_var("AUTH_SECRET", 32)?,
            allowed_origins: var("ALLOWED_ORIGINS")?
                .split(',')
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect(),
            cookie_secure: std::env::var("COOKIE_SECURE").map_or(true, |v| v != "false"),
            turnstile_secret: std::env::var("TURNSTILE_SECRET")
                .ok()
                .filter(|s| !s.is_empty()),
        })
    }
}
