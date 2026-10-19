//! 由 `abi/*.json` 生成的合约绑定（`abi/regen.sh` 从 `contracts/out` 生成；改合约后重跑并提交）。
//!
//! `Escrow.getOrder` 不在这里：它返回的 `Order` 有 27 个字段，超过 `sol!` 元组 24 个的上限，
//! 改由 [`crate::snapshot`] 用 dyn-abi 解码。

#![allow(clippy::too_many_arguments)]

alloy::sol!(
    #[sol(rpc, abi, all_derives)]
    Escrow,
    "abi/Escrow.json"
);

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
