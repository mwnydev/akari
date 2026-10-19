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
            .map_err(|e| MailError::Transport(e.to_string()))?;
        Ok(())
    }
}
