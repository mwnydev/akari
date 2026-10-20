//! Resend API 通道。

use resend_rs::Resend;
use resend_rs::idempotent::IdempotentTrait;
use resend_rs::types::CreateEmailBaseOptions;

use crate::{Email, MailError, Mailer};

pub struct ResendMailer {
    client: Resend,
    from: String,
}

impl ResendMailer {
    /// `from` 形如 `品牌名 <noreply@example.com>`。
    pub fn new(api_key: &str, from: impl Into<String>) -> Self {
        Self {
            client: Resend::new(api_key),
            from: from.into(),
        }
    }
}

impl Mailer for ResendMailer {
    fn name(&self) -> &'static str {
        "resend"
    }

    async fn send(&self, email: &Email) -> Result<(), MailError> {
        let msg = CreateEmailBaseOptions::new(&self.from, [email.to.as_str()], &email.subject)
            .with_html(&email.html)
            .with_text(&email.text);
        // 单封发送接口不支持幂等键，用只含一封的批量接口发送。
        self.client
            .batch
            .send(vec![msg].with_idempotency_key(&email.idempotency_key))
            .await
            .map_err(transport)?;
        Ok(())
    }
}

/// 服务商返回的说明里可能带收件人地址，只留状态码与错误名；按退避重试（Resend 的校验错误分不清是这封邮件的问题
/// 还是发件配置的问题，不当作拒收）。
fn transport(e: resend_rs::Error) -> MailError {
    MailError::Transport(match e {
        resend_rs::Error::Resend(r) => format!("resend: HTTP {} {}", r.status_code, r.name),
        resend_rs::Error::Http(e) => format!("resend: {}", e.without_url()),
        resend_rs::Error::Parse { .. } => "resend: unreadable response".to_owned(),
        resend_rs::Error::RateLimit { .. } => "resend: rate limited".to_owned(),
        other => format!("resend: {other}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_keep_status_and_name_only() {
        let e = transport(resend_rs::Error::Resend(resend_rs::types::ErrorResponse {
            status_code: 422,
            message: "Invalid `to` field: a@example.com".into(),
            name: "validation_error".into(),
        }));
        assert_eq!(
            e.to_string(),
            "transport: resend: HTTP 422 validation_error"
        );
        assert!(!e.is_permanent());
    }
}
