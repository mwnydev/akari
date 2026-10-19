//! 领域核心：金额、费用计算、人民币汇率换算、分期解锁、错误码、邮箱归一化、用户名规则、报价 EIP-712 哈希与条款范围、聊天与通知结构。
//!
//! 规则与 `test-vectors/fees.json` 的 `rules` 一一对应，合约与前端用同一份向量测试。

pub mod chat;
pub mod cny;
pub mod eip712;
pub mod email;
pub mod error;
pub mod fees;
pub mod money;
pub mod trade;
pub mod username;

pub use cny::{cny_fen_to_usdt, usdt_to_cny_fen};
pub use email::normalize_email;
pub use error::ErrorCode;
pub use fees::{
    FeeParams, FeePayer, FeeQuote, claim, dispute_fee, quote, stream_net, stream_unlocked,
};
pub use money::Usdt;
