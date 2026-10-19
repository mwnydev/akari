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
    #[error("not found")]
    NotFound,
    #[error("forbidden")]
    Forbidden,
    #[error("topic locked")]
    TopicLocked,
    #[error("content empty")]
    ContentEmpty,
    #[error("content too long")]
    ContentTooLong,
    #[error("invalid title")]
    InvalidTitle,
    #[error("external link not allowed")]
    ExternalLinkForbidden,
    #[error("order amount out of range")]
    AmountOutOfRange,
    #[error("window out of range")]
    WindowOutOfRange,
    #[error("invalid bond")]
    BondInvalid,
    #[error("invalid fee limits")]
    FeeLimitInvalid,
    #[error("invalid offer expiry")]
    OfferExpiryInvalid,
    #[error("offer no longer available")]
    OfferUnavailable,
    #[error("already exists")]
    Conflict,
    #[error("username required")]
    UsernameRequired,
    #[error("invalid username")]
    InvalidUsername,
    #[error("username taken")]
    UsernameTaken,
    #[error("username changed too recently")]
    UsernameChangeTooSoon,
    #[error("board does not allow trade listings")]
    TradeNotAllowed,
    #[error("unsupported image")]
    UnsupportedImage,
    #[error("image too large")]
    ImageTooLarge,
    #[error("image uploads unavailable")]
    UploadsUnavailable,
    #[error("email re-verification required")]
    ReverifyRequired,
    #[error("passkey verification required")]
    PasskeyRequired,
    #[error("passkey verification failed")]
    PasskeyFailed,
    #[error("too many passkeys")]
    TooManyPasskeys,
    #[error("wallet required")]
    WalletRequired,
    #[error("wallet address does not match the passkey")]
    WalletAddressMismatch,
    #[error("operation not eligible for sponsorship")]
    SponsorNotAllowed,
    #[error("sponsorship quota exceeded")]
    SponsorQuotaExceeded,
    #[error("another sponsored operation is pending")]
    SponsorPending,
    #[error("sponsorship suspended")]
    SponsorSuspended,
    #[error("operation simulation failed")]
    SimulationFailed,
    #[error("sponsor service unavailable")]
    SponsorUnavailable,
    #[error("invalid withdraw address")]
    WithdrawAddressInvalid,
    #[error("withdraw address not allowed")]
    WithdrawAddressForbidden,
    #[error("too many withdraw addresses")]
    WithdrawAddressLimit,
    #[error("withdraw address not active")]
    WithdrawAddressNotActive,
    #[error("no sponsored withdrawal credit left")]
    WithdrawCreditRequired,
    #[error("chain read unavailable")]
    ChainUnavailable,
    #[error("blocked")]
    UserBlocked,
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
            Self::NotFound => "内容不存在或已被删除",
            Self::Forbidden => "没有权限进行此操作",
            Self::TopicLocked => "主题已锁定，不能回复或编辑",
            Self::ContentEmpty => "内容不能为空",
            Self::ContentTooLong => "内容太长",
            Self::InvalidTitle => "标题需为 1–120 个字",
            Self::ExternalLinkForbidden => "注册 24 小时内不能发站外链接",
            Self::AmountOutOfRange => "订单金额超出允许范围",
            Self::WindowOutOfRange => "发货期限和确认期需在 24 小时到 30 天之间",
            Self::BondInvalid => "保证金不能超过订单金额，违约比例不能超过 100%",
            Self::FeeLimitInvalid => "最高费率或最高 gas 费不能低于当前收费，也不能超过合约上限",
            Self::OfferExpiryInvalid => "报价有效期不正确",
            Self::OfferUnavailable => "报价已失效",
            Self::Conflict => "已经存在，不能重复创建",
            Self::UsernameRequired => "请先设置用户名",
            Self::InvalidUsername => {
                "用户名需为 2–20 个字，只能包含汉字、字母、数字和下划线，不能是纯数字或保留名"
            }
            Self::UsernameTaken => "用户名已被使用",
            Self::UsernameChangeTooSoon => "用户名 30 天内只能修改一次",
            Self::TradeNotAllowed => "这个板块不能发交易帖",
            Self::UnsupportedImage => "只支持 JPG、PNG、GIF、WebP、HEIC 图片",
            Self::ImageTooLarge => "图片不能超过 10 MB",
            Self::UploadsUnavailable => "图片上传暂时不可用，请稍后再试",
            Self::ReverifyRequired => "请先完成邮箱验证",
            Self::PasskeyRequired => "请先用钱包 Passkey 确认",
            Self::PasskeyFailed => "Passkey 验证失败，请重试",
            Self::TooManyPasskeys => "Passkey 数量已达上限",
            Self::WalletRequired => "请先开通交易",
            Self::WalletAddressMismatch => "钱包地址与 Passkey 不匹配，请刷新后重试",
            Self::SponsorNotAllowed => "这项操作不在平台代付范围内",
            Self::SponsorQuotaExceeded => "这项操作的代付次数已用完，请稍后再试",
            Self::SponsorPending => "上一笔操作还在上链，请稍候再试",
            Self::SponsorSuspended => "链上失败次数过多，代付已暂停 24 小时",
            Self::SimulationFailed => "交易预演失败，请检查余额和订单状态",
            Self::SponsorUnavailable => "代付服务暂时不可用，请稍后再试",
            Self::WithdrawAddressInvalid => "地址格式不正确，请检查后重新粘贴",
            Self::WithdrawAddressForbidden => "不能使用本人钱包地址或平台合约地址",
            Self::WithdrawAddressLimit => "最多保存 10 个提币地址",
            Self::WithdrawAddressNotActive => "只能提到已生效的提币地址",
            Self::WithdrawCreditRequired => {
                "免费代付提币次数已用完：完成一笔担保交易后可再获得 1 次；也可在「紧急取回」页自付 ETH 提取"
            }
            Self::ChainUnavailable => "暂时无法读取链上数据，请稍后再试",
            Self::UserBlocked => "你已屏蔽对方或被对方屏蔽，无法发送私信",
            Self::Internal => "服务暂时不可用，请稍后再试",
        }
    }
}
