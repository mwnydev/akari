//! 仲裁定时任务（plan 第 7 节、ADR 0013）：分配与超时改派、申诉期到期转为可执行、`resolveTimeout` 临近提醒
//! （规则见 `forum_db::arbitration::tick`），以及站长提醒邮件的内容。

use std::time::Duration;

use anyhow::Context;
use forum_core::arbitration::alert_title;
use forum_db::PgPool;
use forum_mail::templates::{self, Rendered};
use serde::Deserialize;
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// 检查间隔。
pub const INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy)]
pub struct Config {
    /// 接案后（举证期结束起算）多久不裁决就改派。
    pub ruling_deadline: time::Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ruling_deadline: forum_core::arbitration::DEFAULT_RULING_DEADLINE,
        }
    }
}

impl Config {
    /// `ARBITRATION_RULING_DEADLINE_HOURS`（默认 48，1–336）。
    pub fn from_env() -> anyhow::Result<Self> {
        let Some(v) = std::env::var("ARBITRATION_RULING_DEADLINE_HOURS")
            .ok()
            .filter(|v| !v.is_empty())
        else {
            return Ok(Self::default());
        };
        let hours: i64 = v
            .parse()
            .context("ARBITRATION_RULING_DEADLINE_HOURS must be a number")?;
        anyhow::ensure!(
            (1..=336).contains(&hours),
            "ARBITRATION_RULING_DEADLINE_HOURS must be 1–336"
        );
        Ok(Self {
            ruling_deadline: time::Duration::hours(hours),
        })
    }
}

/// 跑一轮，返回处理的事项数（只用于日志）。
pub async fn run_once(db: &PgPool, cfg: Config) -> Result<usize, sqlx::Error> {
    let t = forum_db::arbitration::tick(db, OffsetDateTime::now_utc(), cfg.ruling_deadline).await?;
    Ok(t.total())
}

#[derive(Deserialize)]
struct Alert {
    to: String,
    event: String,
    case_id: i64,
    #[serde(default)]
    detail: Value,
}

fn percent(bps: &Value) -> Option<String> {
    let b = bps.as_i64()?;
    Some(format!("{}.{:02}%", b / 100, b % 100))
}

fn lines(detail: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(p) = percent(&detail["to_buyer_bps"]) {
        let bond = percent(&detail["bond_to_buyer_bps"]).unwrap_or_default();
        out.push(format!("判给买家：本金 {p}，保证金 {bond}；其余归卖家"));
    }
    if let Some(r) = detail["reason"].as_str() {
        out.push(format!("理由：{r}"));
    }
    if let Some(links) = detail["evidence"].as_array() {
        out.extend(
            links
                .iter()
                .filter_map(Value::as_str)
                .map(|l| format!("证据：{l}")),
        );
    }
    if let Some(t) = detail["timeout_at"]
        .as_i64()
        .and_then(|t| OffsetDateTime::from_unix_timestamp(t).ok())
        .and_then(|t| t.format(&Rfc3339).ok())
    {
        out.push(format!("超时兜底时间：{t}"));
    }
    if !detail["executed"].is_null() {
        out.push(format!(
            "链上执行（买家比例, 保证金比例）：{}；已公示：{}",
            detail["executed"], detail["published"]
        ));
    }
    if let Some(s) = detail["status"].as_str() {
        out.push(format!("案件状态：{s}"));
    }
    out
}

/// 站长提醒邮件 `(收件人, 内容)`。
pub(crate) fn alert_email(payload: &Value) -> Result<(String, Rendered), serde_json::Error> {
    let a: Alert = serde_json::from_value(payload.clone())?;
    let rendered =
        templates::arbitration_alert(a.case_id, alert_title(&a.event), &lines(&a.detail));
    Ok((a.to, rendered))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use serde_json::json;

    #[test]
    fn published_alert_lists_split_reason_and_evidence() {
        let (to, (subject, _, text)) = super::alert_email(&json!({
            "to": "owner@example.com", "event": "published", "case_id": 3,
            "detail": {"to_buyer_bps": 7550, "bond_to_buyer_bps": 0, "reason": "卖家未发货",
                       "evidence": ["https://img.example/a.webp"], "status": "appeal_window"},
        }))
        .unwrap();
        assert_eq!(to, "owner@example.com");
        assert!(subject.contains("#3") && subject.contains("裁决已公示"));
        assert!(text.contains("本金 75.50%，保证金 0.00%"));
        assert!(
            text.contains("理由：卖家未发货") && text.contains("证据：https://img.example/a.webp")
        );
    }
}
