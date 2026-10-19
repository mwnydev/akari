//! 由 `abi/*.json` 生成的合约绑定（`abi/regen.sh` 从 `contracts/out` 生成；改合约后重跑并提交）。
//!
//! 两份担保合约的 `getOrder` 不在这里：返回的 `Order` 有 27 / 29 个字段，超过 `sol!` 元组 24 个的上限，
//! 改由 [`crate::snapshot`] 用 dyn-abi 解码。

#![allow(clippy::too_many_arguments)]

alloy::sol!(
    #[sol(rpc, abi, all_derives)]
    Escrow,
    "abi/Escrow.json"
);

/// 分期担保（plan 第 6 节「两种模式」）。ABI 里也有 `FeeMath` 等共用类型，单独一个模块避免与 `Escrow` 的重名。
pub mod stream {
    #![allow(clippy::too_many_arguments)]

    alloy::sol!(
        #[sol(rpc, abi, all_derives)]
        StreamEscrow,
        "abi/StreamEscrow.json"
    );
}

pub use stream::StreamEscrow;

alloy::sol!(
    #[sol(rpc, abi, all_derives)]
    EscrowConfig,
    "abi/EscrowConfig.json"
);

alloy::sol!(
    /// 仅测试链使用的 6 位精度 USDT 替身。
    #[sol(rpc, abi, all_derives)]
    MockUSDT,
    "abi/MockUSDT.json"
);

alloy::sol!(
    /// 自写固定费 Paymaster（ADR 0004）。
    #[sol(rpc, abi, all_derives)]
    FixedFeePaymaster,
    "abi/FixedFeePaymaster.json"
);
