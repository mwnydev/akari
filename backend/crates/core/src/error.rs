//! 对外统一错误码；OpenAPI 与前端中文提示由此生成。

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    #[error("amount overflow")]
    AmountOverflow,
    #[error("amount underflow")]
    AmountUnderflow,
    #[error("invalid parameter")]
    InvalidParameter,
    #[error("claimable amount does not exceed claim gas fee")]
    ClaimBelowGas,
    #[error("invalid email")]
    InvalidEmail,
    #[error("captcha failed")]
    CaptchaFailed,
    #[error("resend too soon")]
    ResendTooSoon,
    #[error("rate limited")]
    RateLimited,
    #[error("invalid or expired code")]
    InvalidCode,
    #[error("too many attempts")]
    TooManyAttempts,
    #[error("not logged in")]
    Unauthorized,
    #[error("origin rejected")]
    OriginRejected,
    #[error("internal error")]
    Internal,
}

impl ErrorCode {
    /// 展示给用户的中文提示；不包含任何内部信息。
    pub fn message(self) -> &'static str {
        match self {
            Self::AmountOverflow | Self::AmountUnderflow => "金额超出范围",
            Self::InvalidParameter => "参数不正确",
            Self::ClaimBelowGas => "可领取金额不足以支付领取 gas 费",
            Self::InvalidEmail => "邮箱格式不正确",
            Self::CaptchaFailed => "人机验证未通过，请重试",
            Self::ResendTooSoon => "验证码发送太频繁，请 60 秒后再试",
            Self::RateLimited => "操作太频繁，请稍后再试",
            Self::InvalidCode => "验证码错误或已过期",
            Self::TooManyAttempts => "错误次数过多，请重新获取验证码",
            Self::Unauthorized => "请先登录",
            Self::OriginRejected => "请求来源不被允许",
            Self::Internal => "服务暂时不可用，请稍后再试",
        }
    }
}
