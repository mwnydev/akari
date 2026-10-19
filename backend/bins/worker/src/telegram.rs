//! 管理员 Telegram 群告警（ADR 0013 第 6 条）：发件箱 `staff_alert` 消息经 Bot API `sendMessage` 推送。
//!
//! 机器人令牌是请求地址的一部分：日志与错误信息里一律去掉地址（`reqwest::Error::without_url`），
//! `Debug` 也不打印。失败由发件箱按指数退避重试。

use std::time::Duration;

use anyhow::bail;
use serde::Deserialize;
use serde_json::json;

/// Bot API 正文上限 4096 字符，留出余量。
const MAX_TEXT_CHARS: usize = 4000;
const TIMEOUT: Duration = Duration::from_secs(10);
pub const API_BASE: &str = "https://api.telegram.org";

pub struct Telegram {
    client: reqwest::Client,
    /// 含令牌的完整 `sendMessage` 地址，不能出现在任何日志里。
    url: String,
    chat_id: String,
}

impl std::fmt::Debug for Telegram {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Telegram")
            .field("chat_id", &self.chat_id)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct ApiError {
    description: Option<String>,
}

impl Telegram {
    /// `api_base` 生产为 [`API_BASE`]（测试指向本机假服务）。
    pub fn new(api_base: &str, token: &str, chat_id: &str) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder().timeout(TIMEOUT).build()?;
        Ok(Self {
            client,
            url: format!("{}/bot{token}/sendMessage", api_base.trim_end_matches('/')),
            chat_id: chat_id.to_owned(),
        })
    }

    /// `TELEGRAM_BOT_TOKEN` 与 `TELEGRAM_ADMIN_CHAT_ID` 都设置才启用；都不设置返回 `None`（告警关闭），只设一个拒绝启动。
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        let get = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        match (get("TELEGRAM_BOT_TOKEN"), get("TELEGRAM_ADMIN_CHAT_ID")) {
            (Some(token), Some(chat)) => Ok(Some(Self::new(API_BASE, token.trim(), chat.trim())?)),
            (None, None) => Ok(None),
            _ => bail!("TELEGRAM_BOT_TOKEN and TELEGRAM_ADMIN_CHAT_ID must be set together"),
        }
    }

    /// 发一条纯文本消息（不解析 Markdown / HTML，用户名等内容原样显示）。错误信息不含令牌。
    pub async fn send(&self, text: &str) -> Result<(), String> {
        let text: String = text.chars().take(MAX_TEXT_CHARS).collect();
        let res = self
            .client
            .post(&self.url)
            .json(
                &json!({ "chat_id": self.chat_id, "text": text, "disable_web_page_preview": true }),
            )
            .send()
            .await
            .map_err(|e| format!("telegram: {}", e.without_url()))?;
        let status = res.status();
        if status.is_success() {
            return Ok(());
        }
        let description = res
            .json::<ApiError>()
            .await
            .ok()
            .and_then(|e| e.description)
            .unwrap_or_default();
        Err(format!("telegram {status}: {description}"))
    }
}
