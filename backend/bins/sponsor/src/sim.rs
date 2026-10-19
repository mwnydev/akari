//! 签字前模拟与 gas 估算（plan 第 5 节「签字前必模拟」）。
//!
//! 用 `eth_simulateV1` 在 latest 状态上按顺序重放这笔操作在 EntryPoint 里会做的事：
//! 1. 有 `initCode` 时调用工厂部署钱包（返回的地址必须就是 sender）；
//! 2. 收费时以 Paymaster 身份 `transferFrom(sender → Paymaster, fee)`，与验证阶段扣费完全相同，
//!    授权或余额不足在这里就暴露；
//! 3. 以 EntryPoint 身份调用 sender 执行 `callData`。
//!
//! 不跑账户签名校验（用户还没签）；那一步由代发器提交前的 `handleOps` 预演覆盖。

use alloy::primitives::{Address, Bytes, U256};
use alloy::sol_types::{SolCall, SolValue};
use forum_chain::aa::{IEntryPoint, PackedUserOperation, UserOperation};
use forum_core::ErrorCode;
use serde::Serialize;

use crate::chain::{Chain, call_request};
use crate::config::Policy;
use crate::jsonrpc::{Reject, Result};

alloy::sol! {
    interface IERC20TransferFrom {
        function transferFrom(address from, address to, uint256 amount) external returns (bool);
    }
}

/// Paymaster 验证阶段：ECDSA 恢复 + 一次 USDT `transferFrom`（USD₮0 是代理，冷读要多留）。不实现 postOp。
pub const PM_VERIFICATION_GAS: u128 = 120_000;
pub const PM_POST_OP_GAS: u128 = 0;
/// Kernel v3.3 + WebAuthn 验证器校验一次 Passkey 签名（走 P-256 预编译）的预算。验证 gas 不按未用部分罚款，
/// 给足即可。
pub const ACCOUNT_VERIFICATION_GAS: u64 = 250_000;
/// 估算请求里还没有账户签名时，按这么长的 WebAuthn 签名计算 calldata 成本。
const STUB_SIGNATURE_LEN: usize = 800;

/// 签名前的 gas 上限检查。
pub const MAX_VERIFICATION_GAS: u64 = 2_000_000;
pub const MAX_CALL_GAS: u64 = 3_000_000;
pub const MAX_PRE_VERIFICATION_GAS: u64 = 5_000_000;
/// `maxFeePerGas` 最多是当前 gas 价格的几倍（报价给出的 fast 档是 2 倍）。
const MAX_FEE_MULTIPLIER: u128 = 3;
/// gas 价格极低的链上，允许的 `maxFeePerGas` 至少到 0.1 gwei。
const MIN_FEE_CAP: u128 = 100_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Simulation {
    /// 部署钱包用掉的 gas（不部署为 0）。
    pub deploy_gas: u64,
    /// 执行 `callData` 用掉的 gas（含 21000 基础费，作为余量）。
    pub exec_gas: u64,
}

fn failed(step: &str, data: &Bytes) -> Reject {
    let mut hex = data.to_string();
    hex.truncate(2 + 2 * 68);
    Reject::new(
        ErrorCode::SimulationFailed,
        format!("{step} reverted: {hex}"),
    )
}

pub async fn simulate(
    chain: &Chain,
    op: &UserOperation,
    paymaster: Address,
    token: Address,
    fee: u128,
) -> Result<Simulation> {
    let deployed = chain.code_len(op.sender).await? > 0;
    let deploying = op.factory.is_some_and(|f| !f.is_zero());
    match (deploying, deployed) {
        (true, true) => {
            return Err(Reject::invalid("account already deployed: drop factory").into());
        }
        (false, false) => return Err(Reject::invalid("account not deployed and no factory").into()),
        _ => {}
    }
    let mut calls = Vec::with_capacity(3);
    if let Some(factory) = op.factory.filter(|_| deploying) {
        let data = op.factory_data.clone().unwrap_or_default();
        calls.push(call_request(chain.entry_point, factory, data));
    }
    if fee > 0 {
        let data = IERC20TransferFrom::transferFromCall {
            from: op.sender,
            to: paymaster,
            amount: U256::from(fee),
        }
        .abi_encode();
        calls.push(call_request(paymaster, token, data.into()));
    }
    calls.push(call_request(
        chain.entry_point,
        op.sender,
        op.call_data.clone(),
    ));

    let results = chain
        .simulate(calls)
        .await?
        .map_err(|e| anyhow::anyhow!("eth_simulateV1: {e}"))?;
    let steps: Vec<&str> = [
        deploying.then_some("deploy"),
        (fee > 0).then_some("fee transfer"),
        Some("callData"),
    ]
    .into_iter()
    .flatten()
    .collect();
    if results.len() != steps.len() {
        return Err(anyhow::anyhow!("eth_simulateV1 returned {} results", results.len()).into());
    }
    for (step, r) in steps.iter().zip(&results) {
        if !r.status {
            return Err(failed(step, &r.return_data).into());
        }
    }
    let mut deploy_gas = 0;
    if deploying {
        let created = Address::abi_decode(&results[0].return_data).ok();
        if created != Some(op.sender) {
            return Err(Reject::not_allowed("factory does not deploy the sender").into());
        }
        deploy_gas = results[0].gas_used;
    }
    Ok(Simulation {
        deploy_gas,
        exec_gas: results.last().map_or(0, |r| r.gas_used),
    })
}

/// `eth_estimateUserOperationGas` 的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GasEstimate {
    pub pre_verification_gas: U256,
    pub verification_gas_limit: U256,
    pub call_gas_limit: U256,
    pub paymaster_verification_gas_limit: U256,
    pub paymaster_post_op_gas_limit: U256,
}

fn calldata_gas(b: &[u8]) -> u64 {
    b.iter().map(|&x| if x == 0 { 4 } else { 16 }).sum()
}

/// 与常见 bundler 相同的 v0.7 算法：交易基础费 + 每笔固定开销 + 打包后 calldata 成本。
/// 代发器自己是 beneficiary，这部分从 Paymaster 押金流向热钱包，只需大致补上热钱包的成本。
pub fn pre_verification_gas(packed: &PackedUserOperation) -> u64 {
    let encoded = packed.abi_encode();
    let words = encoded.len().div_ceil(32) as u64;
    21_000 + 18_300 + 4 * words + calldata_gas(&encoded)
}

pub async fn estimate(
    chain: &Chain,
    op: &UserOperation,
    sim: Simulation,
    beneficiary: Address,
) -> Result<GasEstimate> {
    let mut sized = op.clone();
    if sized.signature.is_empty() {
        sized.signature = vec![0xff; STUB_SIGNATURE_LEN].into();
    }
    let packed = sized.pack()?;
    let handle_ops = IEntryPoint::handleOpsCall {
        ops: vec![packed.clone()],
        beneficiary,
    }
    .abi_encode();
    // L1 数据费价格波动大，多留 25%。
    let l1 = chain.l1_gas(handle_ops.into()).await? * 5 / 4;
    let verification = ACCOUNT_VERIFICATION_GAS + sim.deploy_gas * 6 / 5;
    Ok(GasEstimate {
        pre_verification_gas: U256::from(pre_verification_gas(&packed) + l1),
        verification_gas_limit: U256::from(verification),
        call_gas_limit: U256::from(sim.exec_gas * 6 / 5 + 10_000),
        paymaster_verification_gas_limit: U256::from(PM_VERIFICATION_GAS),
        paymaster_post_op_gas_limit: U256::from(PM_POST_OP_GAS),
    })
}

/// 签名前检查 gas 字段：上限、价格不离谱、押金最多承担 `max_op_cost`。
pub fn check_limits(op: &UserOperation, policy: &Policy, gas_price: u128) -> Result<()> {
    let too_high = |what: &str| Err(Reject::not_allowed(format!("{what} too high")).into());
    if op.verification_gas_limit > U256::from(MAX_VERIFICATION_GAS) {
        return too_high("verificationGasLimit");
    }
    if op.call_gas_limit > U256::from(MAX_CALL_GAS) {
        return too_high("callGasLimit");
    }
    if op.pre_verification_gas > U256::from(MAX_PRE_VERIFICATION_GAS) {
        return too_high("preVerificationGas");
    }
    if op.call_gas_limit.is_zero() || op.verification_gas_limit.is_zero() {
        return Err(Reject::invalid("gas limits must be set before sponsoring").into());
    }
    if op.max_priority_fee_per_gas > op.max_fee_per_gas {
        return Err(Reject::invalid("maxPriorityFeePerGas exceeds maxFeePerGas").into());
    }
    let cap = gas_price
        .saturating_mul(MAX_FEE_MULTIPLIER)
        .max(MIN_FEE_CAP);
    if op.max_fee_per_gas > U256::from(cap) {
        return too_high("maxFeePerGas");
    }
    if op.max_cost()? > policy.max_op_cost {
        return too_high("total gas cost");
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GasPrice {
    pub max_fee_per_gas: U256,
    pub max_priority_fee_per_gas: U256,
}

/// `pimlico_getUserOperationGasPrice` 兼容格式：slow / standard / fast = 当前 gas 价格的 1.2 / 1.5 / 2 倍。
pub fn price_tiers(gas_price: u128, priority: u128) -> serde_json::Value {
    let tier = |num: u128, den: u128| {
        let max_fee = gas_price.saturating_mul(num) / den;
        GasPrice {
            max_fee_per_gas: U256::from(max_fee),
            max_priority_fee_per_gas: U256::from(priority.min(max_fee)),
        }
    };
    serde_json::json!({"slow": tier(6, 5), "standard": tier(3, 2), "fast": tier(2, 1)})
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn op() -> UserOperation {
        UserOperation {
            verification_gas_limit: U256::from(300_000),
            call_gas_limit: U256::from(200_000),
            pre_verification_gas: U256::from(60_000),
            max_fee_per_gas: U256::from(20_000_000u64),
            max_priority_fee_per_gas: U256::from(0),
            ..UserOperation::default()
        }
    }

    fn policy() -> Policy {
        Policy {
            sponsor_fee: 50_000,
            max_op_cost: U256::from(1_000_000_000_000_000u64),
        }
    }

    #[test]
    fn limits_accept_a_normal_operation() {
        check_limits(&op(), &policy(), 10_000_000).unwrap();
    }

    #[test]
    fn limits_reject_excessive_fields() {
        let cases: [fn(&mut UserOperation); 6] = [
            |o| o.verification_gas_limit = U256::from(MAX_VERIFICATION_GAS + 1),
            |o| o.call_gas_limit = U256::from(MAX_CALL_GAS + 1),
            |o| o.pre_verification_gas = U256::from(MAX_PRE_VERIFICATION_GAS + 1),
            |o| o.call_gas_limit = U256::ZERO,
            |o| o.max_priority_fee_per_gas = o.max_fee_per_gas + U256::from(1),
            |o| o.max_fee_per_gas = U256::from(200_000_000u64),
        ];
        for (i, mutate) in cases.iter().enumerate() {
            let mut o = op();
            mutate(&mut o);
            assert!(check_limits(&o, &policy(), 10_000_000).is_err(), "case {i}");
        }
        let tight = Policy {
            max_op_cost: U256::from(1),
            ..policy()
        };
        assert!(check_limits(&op(), &tight, 10_000_000).is_err());
    }

    #[test]
    fn fee_cap_has_a_floor_on_cheap_chains() {
        let mut o = op();
        o.max_fee_per_gas = U256::from(MIN_FEE_CAP);
        check_limits(&o, &policy(), 1).unwrap();
    }

    #[test]
    fn pre_verification_gas_grows_with_calldata() {
        let small = op().pack().unwrap();
        let mut big = op();
        big.call_data = vec![1u8; 1000].into();
        let big = big.pack().unwrap();
        assert!(pre_verification_gas(&big) >= pre_verification_gas(&small) + 16_000);
    }

    #[test]
    fn price_tiers_are_ordered() {
        let v = price_tiers(1_000, 5_000);
        assert_eq!(v["slow"]["maxFeePerGas"], "0x4b0");
        assert_eq!(v["standard"]["maxFeePerGas"], "0x5dc");
        assert_eq!(v["fast"]["maxFeePerGas"], "0x7d0");
        assert_eq!(
            v["fast"]["maxPriorityFeePerGas"], "0x7d0",
            "priority clamped to max fee"
        );
    }
}
