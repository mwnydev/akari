//! 统一「发邮件」接口（plan 第 12 节）：Resend API 与任意 SMTP 两种实现，主通道失败自动切到备用。
//!
//! 只由 worker 经发件箱调用；日志里不记录收件人地址和正文。

pub mod resend;
pub mod smtp;
pub mod templates;

use std::future::Future;

pub use resend::ResendMailer;
pub use smtp::SmtpMailer;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Email {
    pub to: String,
    pub subject: String,
    pub html: String,
    pub text: String,
    /// 同一个键重复发送时服务商只投递一次（发件箱 ID），避免重试造成重复邮件。
    pub idempotency_key: String,
}

#[derive(Debug, thiserror::Error)]
pub enum MailError {
    #[error("invalid address: {0}")]
    Address(String),
    #[error("build message: {0}")]
    Build(String),
    #[error("transport: {0}")]
    Transport(String),
}

pub trait Mailer: Send + Sync {
    /// 通道名，用于日志和告警。
    fn name(&self) -> &'static str;
    fn send(&self, email: &Email) -> impl Future<Output = Result<(), MailError>> + Send;
}

/// 运行时选择的通道。
pub enum AnyMailer {
    Resend(ResendMailer),
    Smtp(SmtpMailer),
}

impl Mailer for AnyMailer {
    fn name(&self) -> &'static str {
        match self {
            Self::Resend(m) => m.name(),
            Self::Smtp(m) => m.name(),
        }
    }

    async fn send(&self, email: &Email) -> Result<(), MailError> {
        match self {
            Self::Resend(m) => m.send(email).await,
            Self::Smtp(m) => m.send(email).await,
        }
    }
}

/// 主通道失败时改走备用通道；两个都失败才返回错误（由发件箱按指数退避重试）。
pub struct Failover<P, B> {
    primary: P,
    backup: Option<B>,
}

impl<P: Mailer, B: Mailer> Failover<P, B> {
    pub fn new(primary: P, backup: Option<B>) -> Self {
        Self { primary, backup }
    }
}

impl<P: Mailer, B: Mailer> Mailer for Failover<P, B> {
    fn name(&self) -> &'static str {
        "failover"
    }

    async fn send(&self, email: &Email) -> Result<(), MailError> {
        match self.primary.send(email).await {
            Ok(()) => Ok(()),
            Err(primary_err) => {
                let Some(backup) = &self.backup else {
                    return Err(primary_err);
                };
                tracing::warn!(
                    channel = self.primary.name(),
                    error = %primary_err,
                    backup = backup.name(),
                    "primary mail channel failed, using backup"
                );
                backup.send(email).await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    struct Fake {
        ok: bool,
        calls: AtomicUsize,
    }

    impl Fake {
        fn new(ok: bool) -> Self {
            Self {
                ok,
                calls: AtomicUsize::new(0),
            }
        }
    }

    impl Mailer for Fake {
        fn name(&self) -> &'static str {
            "fake"
        }
        async fn send(&self, _: &Email) -> Result<(), MailError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.ok {
                Ok(())
            } else {
                Err(MailError::Transport("down".into()))
            }
        }
    }

    fn email() -> Email {
        Email {
            to: "a@example.com".into(),
            subject: "s".into(),
            html: "h".into(),
            text: "t".into(),
            idempotency_key: "outbox-1".into(),
        }
    }

    #[tokio::test]
    async fn primary_ok_skips_backup() {
        let m = Failover::new(Fake::new(true), Some(Fake::new(true)));
        m.send(&email()).await.unwrap();
        assert_eq!(m.backup.as_ref().unwrap().calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn primary_down_uses_backup() {
        let m = Failover::new(Fake::new(false), Some(Fake::new(true)));
        m.send(&email()).await.unwrap();
        assert_eq!(m.primary.calls.load(Ordering::SeqCst), 1);
        assert_eq!(m.backup.as_ref().unwrap().calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn both_down_is_error() {
        let m = Failover::new(Fake::new(false), Some(Fake::new(false)));
        assert!(m.send(&email()).await.is_err());
        let m: Failover<Fake, Fake> = Failover::new(Fake::new(false), None);
        assert!(m.send(&email()).await.is_err());
    }
}
