//! 链上读写：alloy 合约绑定、链配置、事件解码、订单状态折叠与区块回滚判定。
//!
//! 合约接口的唯一定义处是 Solidity 源码（plan 第 14 节）：`abi/*.json` 由 `abi/regen.sh` 从 Foundry 产物生成，
//! 这里用 `sol!` 宏读取，后端编译不需要安装 Foundry。

#[cfg(any(test, feature = "anvil"))]
pub mod anvil;
pub mod bindings;
pub mod config;
mod error;
pub mod event;
pub mod kernel;
pub mod order;
pub mod reorg;
pub mod rpc;
pub mod snapshot;

pub use error::ChainError;
