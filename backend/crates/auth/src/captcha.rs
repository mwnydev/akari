//! 人机验证。生产用 Cloudflare Turnstile；开发与测试用 `NoopCaptcha`。

use std::future::Future;
use std::net::IpAddr;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// 页面渲染组件时传的 `action`，服务端核对令牌是在哪个表单上拿到的：登录表单的令牌不能拿去发帖，反之亦然。
pub mod action {
    /// 请求登录 / 注册验证码。
    pub const LOGIN: &str = "login";
    /// 新用户发主题、回帖、评论。
    pub const POST: &str = "post";
}

pub trait Captcha: Send + Sync {
    /// 返回 `true` 表示通过（且令牌是在 `action` 表单上拿到的）。服务不可用时返回 `Err`，由调用方决定放行还是拒绝
    /// （登录与发帖都拒绝）。
    fn verify(
        &self,
        token: Option<&str>,
        ip: IpAddr,
        action: &str,
    ) -> impl Future<Output = Result<bool, reqwest::Error>> + Send;
}

/// 永远通过。只能在开发环境使用（配置里没有 Turnstile 密钥时启用并打印警告）。
#[derive(Debug, Clone, Default)]
pub struct NoopCaptcha;

impl Captcha for NoopCaptcha {
    async fn verify(
        &self,
        _token: Option<&str>,
        _ip: IpAddr,
        _action: &str,
    ) -> Result<bool, reqwest::Error> {
        Ok(true)
    }
}

/// Cloudflare 的令牌校验接口。
pub const SITEVERIFY: &str = "https://challenges.cloudflare.com/turnstile/v0/siteverify";
/// 校验请求的超时：超时按服务不可用处理（拒绝），不让请求挂住。
pub const TIMEOUT: Duration = Duration::from_secs(5);
/// Turnstile 令牌最长 2048 字符。
const MAX_TOKEN_LEN: usize = 2048;

#[derive(Debug, Clone)]
pub struct Turnstile {
    client: reqwest::Client,
    secret: String,
    endpoint: String,
}

impl Turnstile {
    /// `client` 由调用方建（出网客户端登记在 `forum-runtime` 的出网测试里），应带 [`TIMEOUT`]。
    pub fn new(client: reqwest::Client, secret: String) -> Self {
        Self::with_endpoint(client, secret, SITEVERIFY.to_owned())
    }

    /// 换校验地址：只给测试用（本机的假 siteverify）；生产地址固定为 [`SITEVERIFY`]，不进配置。
    pub fn with_endpoint(client: reqwest::Client, secret: String, endpoint: String) -> Self {
        Self {
            client,
            secret,
            endpoint,
        }
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
    #[serde(default)]
    action: Option<String>,
}

impl Captcha for Turnstile {
    async fn verify(
        &self,
        token: Option<&str>,
        ip: IpAddr,
        action: &str,
    ) -> Result<bool, reqwest::Error> {
        let Some(token) = token.filter(|t| !t.is_empty() && t.len() <= MAX_TOKEN_LEN) else {
            return Ok(false);
        };
        let body = SiteVerifyRequest {
            secret: &self.secret,
            response: token,
            remoteip: ip.to_string(),
        };
        let res: SiteVerify = self
            .client
            .post(&self.endpoint)
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(res.success && res.action.as_deref() == Some(action))
    }
}

/// 运行时选择的人机验证实现。
#[derive(Debug, Clone)]
pub enum AnyCaptcha {
    Noop(NoopCaptcha),
    Turnstile(Turnstile),
}

impl Captcha for AnyCaptcha {
    async fn verify(
        &self,
        token: Option<&str>,
        ip: IpAddr,
        action: &str,
    ) -> Result<bool, reqwest::Error> {
        match self {
            Self::Noop(c) => c.verify(token, ip, action).await,
            Self::Turnstile(c) => c.verify(token, ip, action).await,
        }
    }
}
