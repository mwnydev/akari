//! SMTP 通道（任意服务商；开发环境指向 Mailpit）。SMTP 没有幂等键，极少数情况下重试可能重复投递。

use lettre::message::{Mailbox, MultiPart};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::{Email, MailError, Mailer};

pub struct SmtpMailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}

impl SmtpMailer {
    /// `url` 形如 `smtps://user:pass@smtp.example.com:465`、`smtp://user:pass@host:587?tls=required`，
    /// 开发环境 `smtp://127.0.0.1:1025`。
    pub fn from_url(url: &str, from: &str) -> Result<Self, MailError> {
        let transport = AsyncSmtpTransport::<Tokio1Executor>::from_url(url)
            .map_err(|e| MailError::Transport(e.to_string()))?
            .build();
        let from = from
            .parse()
            .map_err(|_| MailError::Address("from".to_owned()))?;
        Ok(Self { transport, from })
    }
}

impl Mailer for SmtpMailer {
    fn name(&self) -> &'static str {
        "smtp"
    }

    async fn send(&self, email: &Email) -> Result<(), MailError> {
        let to: Mailbox = email
            .to
            .parse()
            .map_err(|_| MailError::Address("to".to_owned()))?;
        let msg = Message::builder()
            .from(self.from.clone())
            .to(to)
            .subject(&email.subject)
            .multipart(MultiPart::alternative_plain_html(
                email.text.clone(),
                email.html.clone(),
            ))
            .map_err(|e| MailError::Build(e.to_string()))?;
        self.transport
            .send(msg)
            .await
            .map_err(|e| MailError::Transport(e.to_string()))?;
        Ok(())
    }
}
