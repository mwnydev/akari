//! 一条发件箱消息投递失败的分类：重试也不会成功的直接进死信，其余按退避重试。

use forum_mail::MailError;

use crate::RenderError;

/// 一次投递失败。`permanent`：重试也不会成功（违反约束、数据格式错误、payload 不对、不认识的类型、服务商拒收），
/// 直接进死信；其余（数据库连接与并发冲突、网络、服务商故障与限流、超时）按退避重试，次数用尽才进死信。
#[derive(Debug)]
pub struct Failure {
    pub permanent: bool,
    error: String,
}

impl Failure {
    pub fn permanent(error: impl Into<String>) -> Self {
        Self {
            permanent: true,
            error: error.into(),
        }
    }

    pub fn transient(error: impl Into<String>) -> Self {
        Self {
            permanent: false,
            error: error.into(),
        }
    }

    /// 写进 `last_error`、日志与告警的错误信息：去掉像邮箱地址的词（服务商的错误说明可能带收件人），截到 300 字。
    pub fn error(&self) -> String {
        let redacted: Vec<&str> = self
            .error
            .split_whitespace()
            .map(|w| if w.contains('@') { "<redacted>" } else { w })
            .collect();
        redacted.join(" ").chars().take(300).collect()
    }
}

impl From<RenderError> for Failure {
    fn from(e: RenderError) -> Self {
        Self::permanent(e.to_string())
    }
}

impl From<MailError> for Failure {
    fn from(e: MailError) -> Self {
        Self {
            permanent: e.is_permanent(),
            error: e.to_string(),
        }
    }
}

/// 数据库错误：违反约束（SQLSTATE 23）、数据格式错误（22）与读出的行对不上代码（解码失败、缺列）重试也一样；
/// 其余（连接、连接池、序列化冲突与死锁、权限）可能自己恢复或修好配置后恢复。错误信息只有 PostgreSQL 的
/// 主消息（约束名等），不含 DETAIL 里的行内容。
impl From<sqlx::Error> for Failure {
    fn from(e: sqlx::Error) -> Self {
        let permanent = match &e {
            sqlx::Error::Database(d) => d
                .code()
                .is_some_and(|c| c.starts_with("22") || c.starts_with("23")),
            sqlx::Error::RowNotFound
            | sqlx::Error::TypeNotFound { .. }
            | sqlx::Error::ColumnIndexOutOfBounds { .. }
            | sqlx::Error::ColumnNotFound(_)
            | sqlx::Error::ColumnDecode { .. }
            | sqlx::Error::Decode(_) => true,
            _ => false,
        };
        Self {
            permanent,
            error: e.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification() {
        assert!(Failure::from(RenderError::UnknownKind).permanent);
        assert!(Failure::from(MailError::Rejected("x".into())).permanent);
        assert!(!Failure::from(MailError::Transport("x".into())).permanent);
        assert!(!Failure::from(sqlx::Error::PoolTimedOut).permanent);
        assert!(Failure::from(sqlx::Error::RowNotFound).permanent);
    }

    #[test]
    fn error_text_is_redacted_and_bounded() {
        let f = Failure::transient("rejected: Invalid 'To' address: 'a@example.com'.");
        assert_eq!(f.error(), "rejected: Invalid 'To' address: <redacted>");
        assert_eq!(Failure::transient("x".repeat(1000)).error().len(), 300);
    }
}
