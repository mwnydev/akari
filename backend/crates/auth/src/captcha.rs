//! 人机验证。生产用 Cloudflare Turnstile；开发与测试用 `NoopCaptcha`。

use std::future::Future;
use std::net::IpAddr;

use serde::{Deserialize, Serialize};

pub trait Captcha: Send + Sync {
    /// 返回 `true` 表示通过。服务不可用时返回 `Err`，由调用方决定放行还是拒绝（登录接口拒绝）。
    fn verify(
        &self,
        token: Option<&str>,
        ip: IpAddr,
    ) -> impl Future<Output = Result<bool, reqwest::Error>> + Send;
}

/// 永远通过。只能在开发环境使用（配置里没有 Turnstile 密钥时启用并打印警告）。
#[derive(Debug, Clone, Default)]
pub struct NoopCaptcha;

impl Captcha for NoopCaptcha {
    async fn verify(&self, _token: Option<&str>, _ip: IpAddr) -> Result<bool, reqwest::Error> {
        Ok(true)
    }
}

const SITEVERIFY: &str = "https://challenges.cloudflare.com/turnstile/v0/siteverify";

#[derive(Debug, Clone)]
pub struct Turnstile {
    client: reqwest::Client,
    secret: String,
}

impl Turnstile {
    pub fn new(client: reqwest::Client, secret: String) -> Self {
        Self { client, secret }
    }
}

#[derive(Serialize)]
struct SiteVerifyRequest<'a> {
    secret: &'a str,
    response: &'a str,
    remoteip: String,
}

#[derive(Deserialize)]
struct SiteVerify {
    success: bool,
}

impl Captcha for Turnstile {
    async fn verify(&self, token: Option<&str>, ip: IpAddr) -> Result<bool, reqwest::Error> {
        let Some(token) = token.filter(|t| !t.is_empty() && t.len() <= 2048) else {
            return Ok(false);
        };
        let body = SiteVerifyRequest {
            secret: &self.secret,
            response: token,
            remoteip: ip.to_string(),
        };
        let res: SiteVerify = self
            .client
            .post(SITEVERIFY)
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(res.success)
    }
}

/// 运行时选择的人机验证实现。
#[derive(Debug, Clone)]
pub enum AnyCaptcha {
    Noop(NoopCaptcha),
    Turnstile(Turnstile),
}

impl Captcha for AnyCaptcha {
    async fn verify(&self, token: Option<&str>, ip: IpAddr) -> Result<bool, reqwest::Error> {
        match self {
            Self::Noop(c) => c.verify(token, ip).await,
            Self::Turnstile(c) => c.verify(token, ip).await,
        }
    }
}
