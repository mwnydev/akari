//! 领域核心：金额、费用计算、分期解锁、错误码。
//!
//! 规则与 `test-vectors/fees.json` 的 `rules` 一一对应，合约与前端用同一份向量测试。

pub mod error;
pub mod fees;
pub mod money;

pub use error::ErrorCode;
pub use fees::{
    FeeParams, FeePayer, FeeQuote, claim, dispute_fee, quote, stream_net, stream_unlocked,
};
pub use money::Usdt;
