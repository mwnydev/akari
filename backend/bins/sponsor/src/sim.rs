//! 签字前模拟与 gas 估算（plan 第 5 节「签字前必模拟」）。
//!
//! 在 latest 状态上按顺序重放这笔操作在 EntryPoint 里会做的事：
//! 1. 有 `initCode` 时部署钱包（得到的地址必须就是 sender）；
//! 2. 以 Paymaster 身份 `transferFrom(sender → Paymaster, fee)`，与验证阶段扣费完全相同，授权或余额不足在这里就暴露；
//!    前后各读一次双方的 USDT 余额，必须恰好一减一增 `fee`，不能有别的代币变动；
//! 3. 以 EntryPoint 身份调用 sender 执行 `callData`。
//!
//! 两种模拟方法，按 RPC 节点选择（[`Router`]）：
//! - `eth_simulateV1`（首选）：上面每一步是一笔调用；
//! - EntryPoint `eth_call`（备用）：Arbitrum Nitro 的 `eth_simulateV1` 会崩溃（`-32603 method handler crashed`，
//!   OffchainLabs/nitro#3372）。用状态覆盖把 EntryPoint 的代码换成 `sim/SponsorSimulations.sol`，一次 `eth_call` 里走
//!   EntryPoint 自己的部署与 Paymaster 验证（押金检查、`validatePaymasterUserOp`、Paymaster gas 上限），再按
//!   `callGasLimit` 执行 `callData`。Paymaster 的 `signer` 槽同时覆盖成一次性密钥，真实代付签名不会发给 RPC 服务商。
//!
//! 某个节点的 `eth_simulateV1` 报「方法不支持」或崩溃时，该节点 [`V1_COOLDOWN`] 内直接用备用方法。其他任何错误、
//! 结果不完整或对不上，一律拒签（不放行）。
//!
//! 不跑账户签名校验（用户还没签）；那一步由代发器提交前的 `handleOps` 预演覆盖。

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use alloy::primitives::{Address, B256, Bytes, U256};
use alloy::rpc::types::TransactionRequest;
use alloy::rpc::types::simulate::SimCallResult;
use alloy::rpc::types::state::{AccountOverride, StateOverride};
use alloy::signers::SignerSync;
use alloy::signers::local::PrivateKeySigner;
use alloy::sol_types::{SolCall, SolInterface, SolValue};
use forum_chain::aa::{IEntryPoint, PackedUserOperation, SponsorTerms, UserOperation};
use forum_core::ErrorCode;
use serde::Serialize;

use crate::chain::{self, Chain, NodeError, call_request};
use crate::config::Policy;
use crate::jsonrpc::{INTERNAL, METHOD_NOT_FOUND, Reject, Result};

alloy::sol! {
    interface IERC20Sim {
        function transferFrom(address from, address to, uint256 amount) external returns (bool);
        function balanceOf(address account) external view returns (uint256);
    }

    /// `sim/SponsorSimulations.sol`（与 [`PackedUserOperation`] 同构，单独声明是因为 `sol!` 不能引用外部类型）。
    interface ISponsorSimulations {
        struct SimUserOperation {
            address sender;
            uint256 nonce;
            bytes initCode;
            bytes callData;
            bytes32 accountGasLimits;
            uint256 preVerificationGas;
            bytes32 gasFees;
            bytes paymasterAndData;
            bytes signature;
        }

        struct Outcome {
            uint256 deployGas;
            uint256 paymasterValidationData;
            uint256 senderBalanceBefore;
            uint256 senderBalanceAfter;
            uint256 paymasterBalanceBefore;
            uint256 paymasterBalanceAfter;
            bool executed;
            uint256 executionGas;
            bytes executionResult;
        }

        function simulateSponsored(SimUserOperation calldata op, address token) external returns (Outcome memory);
    }
}

/// `SponsorSimulations` 的运行时字节码（`sim/regen.sh` 生成并入库，`tests/sim_bytecode.rs` 校验没有过期）。
const SIMULATIONS_CODE: &str = include_str!("../sim/SponsorSimulations.hex");
/// FixedFeePaymaster 的 `signer` 存储槽（`_owner` 在 0 号槽）；启动时与链上 `signer()` 核对（[`crate::App::verify_onchain`]）。
pub const PAYMASTER_SIGNER_SLOT: U256 = U256::from_limbs([1, 0, 0, 0]);

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

/// 节点的 `eth_simulateV1` 不可用之后，多久内这个节点直接用备用方法（之后再试一次 `eth_simulateV1`）。
pub const V1_COOLDOWN: Duration = Duration::from_secs(600);
/// 交易基础费：`eth_simulateV1` 报告的是整笔调用的 gas，备用方法量的是调用内部，补上这部分两者口径一致。
const TX_BASE_GAS: u64 = 21_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    SimulateV1,
    EntryPoint,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SimulateV1 => "eth_simulateV1",
            Self::EntryPoint => "entrypoint_eth_call",
        }
    }
}

/// 每个 RPC 节点用哪种模拟方法，以及各方法的累计使用次数（`/healthz` 显示）。
#[derive(Debug)]
pub struct Router {
    cooldown: Duration,
    /// 节点下标 → 发现 `eth_simulateV1` 不可用的时间。
    v1_unavailable: Mutex<HashMap<usize, Instant>>,
    simulate_v1: AtomicU64,
    entry_point: AtomicU64,
}

impl Default for Router {
    fn default() -> Self {
        Self::new(V1_COOLDOWN)
    }
}

/// [`Router::counts`] 的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MethodCounts {
    pub simulate_v1: u64,
    pub entry_point: u64,
}

impl Router {
    pub fn new(cooldown: Duration) -> Self {
        Self {
            cooldown,
            v1_unavailable: Mutex::new(HashMap::new()),
            simulate_v1: AtomicU64::new(0),
            entry_point: AtomicU64::new(0),
        }
    }

    /// 这个节点现在该不该先试 `eth_simulateV1`。锁中毒时只用备用方法（同样完整校验）。
    fn try_v1(&self, endpoint: usize) -> bool {
        let Ok(mut m) = self.v1_unavailable.lock() else {
            return false;
        };
        match m.get(&endpoint) {
            Some(at) if at.elapsed() < self.cooldown => false,
            Some(_) => {
                m.remove(&endpoint);
                true
            }
            None => true,
        }
    }

    fn mark_v1_unavailable(&self, endpoint: usize) {
        if let Ok(mut m) = self.v1_unavailable.lock() {
            m.insert(endpoint, Instant::now());
        }
    }

    fn record(&self, method: Method) {
        let c = match method {
            Method::SimulateV1 => &self.simulate_v1,
            Method::EntryPoint => &self.entry_point,
        };
        c.fetch_add(1, Ordering::Relaxed);
    }

    pub fn counts(&self) -> MethodCounts {
        MethodCounts {
            simulate_v1: self.simulate_v1.load(Ordering::Relaxed),
            entry_point: self.entry_point.load(Ordering::Relaxed),
        }
    }
}

/// 节点不支持或执行 `eth_simulateV1` 时崩溃（而不是这笔操作本身有问题）：换备用方法。
/// 判断错了也不会放行：备用方法同样完整校验。
fn v1_unavailable(e: &NodeError) -> bool {
    let m = e.message.to_ascii_lowercase();
    match e.code {
        METHOD_NOT_FOUND => true,
        // Nitro：OffchainLabs/nitro#3372。
        INTERNAL => m.contains("method handler crashed"),
        _ => {
            m.contains("method")
                && [
                    "unsupported",
                    "not supported",
                    "does not exist",
                    "not available",
                ]
                .iter()
                .any(|s| m.contains(s))
        }
    }
}

/// 代付的对象：哪个 Paymaster、收哪种代币、按什么条款。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sponsored {
    pub paymaster: Address,
    pub token: Address,
    pub terms: SponsorTerms,
}

/// 备用方法按什么 gas 上限执行。`eth_simulateV1` 不受 gas 字段限制。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limits {
    /// 签名前：就按这笔操作自己的 gas 上限与价格（押金不够、gas 不够都会失败）。
    Signed,
    /// 估算：gas 字段还没定，按允许的最大上限、零 gas 价格执行，量出实际用量。
    Estimate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Simulation {
    /// 部署钱包用掉的 gas（不部署为 0）。
    pub deploy_gas: u64,
    /// 执行 `callData` 用掉的 gas（含 21000 基础费，作为余量）。
    pub exec_gas: u64,
    pub method: Method,
}

/// 一步调用的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Step {
    ok: bool,
    gas: u64,
    data: Bytes,
}

/// 收费前后 sender 与 Paymaster 的代币余额。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Balances {
    sender: (U256, U256),
    paymaster: (U256, U256),
}

/// 两种方法共同的结果，由 [`check`] 统一判定。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Outcome {
    deploy: Option<Step>,
    /// 收费步骤本身（只有 `eth_simulateV1` 单独执行它；备用方法里它在 Paymaster 验证内，失败即整体回滚）。
    fee_call: Option<Step>,
    /// 不收费时 `eth_simulateV1` 不读余额。
    balances: Option<Balances>,
    /// Paymaster 验证返回的 validationData（`eth_simulateV1` 不跑验证，为 0）。
    paymaster_validation: U256,
    exec: Step,
    method: Method,
}

fn failed(step: &str, data: &Bytes) -> Reject {
    let mut hex = data.to_string();
    hex.truncate(2 + 2 * 68);
    Reject::new(
        ErrorCode::SimulationFailed,
        format!("{step} reverted: {hex}"),
    )
}

fn sim_error(msg: impl std::fmt::Display) -> crate::jsonrpc::Error {
    anyhow::anyhow!("simulation: {msg}").into()
}

/// 判定模拟结果；任何一项不符合都拒签。
fn check(o: &Outcome, op: &UserOperation, fee: u128) -> Result<Simulation> {
    if let Some(d) = o.deploy.as_ref().filter(|d| !d.ok) {
        return Err(failed("deploy", &d.data).into());
    }
    if let Some(f) = o.fee_call.as_ref().filter(|f| !f.ok) {
        return Err(failed("fee transfer", &f.data).into());
    }
    let fee = U256::from(fee);
    match o.balances {
        Some(b) => {
            let paid = b.sender.0.checked_sub(b.sender.1);
            let received = b.paymaster.1.checked_sub(b.paymaster.0);
            if paid != Some(fee) || received != Some(fee) {
                return Err(Reject::new(
                    ErrorCode::SimulationFailed,
                    format!(
                        "unexpected token movement during validation: sender {} → {}, paymaster {} → {}, fee {fee}",
                        b.sender.0, b.sender.1, b.paymaster.0, b.paymaster.1
                    ),
                )
                .into());
            }
        }
        None if fee.is_zero() => {}
        None => return Err(sim_error("fee charged but token balances not checked")),
    }
    // 低 160 位非 0 = Paymaster 认为签名无效：一次性密钥没生效（存储布局与预期不符），这是本服务的故障。
    if !Address::from_word(B256::from(o.paymaster_validation)).is_zero() {
        return Err(sim_error("paymaster rejected the simulation signer"));
    }
    if !o.exec.ok {
        return Err(failed("callData", &o.exec.data).into());
    }
    let deploying = op.factory.is_some_and(|f| !f.is_zero());
    if deploying != o.deploy.is_some() {
        return Err(sim_error("deploy step missing or unexpected"));
    }
    Ok(Simulation {
        deploy_gas: o.deploy.as_ref().map_or(0, |d| d.gas),
        exec_gas: o.exec.gas,
        method: o.method,
    })
}

/// `eth_simulateV1` 的调用序列。
struct V1Plan {
    calls: Vec<TransactionRequest>,
    deploying: bool,
    fee: bool,
}

fn v1_plan(chain: &Chain, op: &UserOperation, s: &Sponsored, deploying: bool) -> V1Plan {
    let fee = s.terms.fee > 0;
    let mut calls = Vec::with_capacity(7);
    if let Some(factory) = op.factory.filter(|_| deploying) {
        let data = op.factory_data.clone().unwrap_or_default();
        calls.push(call_request(chain.entry_point, factory, data));
    }
    let balance = |who: Address| {
        let data = IERC20Sim::balanceOfCall { account: who }.abi_encode();
        call_request(Address::ZERO, s.token, data.into())
    };
    if fee {
        calls.push(balance(op.sender));
        calls.push(balance(s.paymaster));
        let data = IERC20Sim::transferFromCall {
            from: op.sender,
            to: s.paymaster,
            amount: U256::from(s.terms.fee),
        }
        .abi_encode();
        calls.push(call_request(s.paymaster, s.token, data.into()));
        calls.push(balance(op.sender));
        calls.push(balance(s.paymaster));
    }
    calls.push(call_request(
        chain.entry_point,
        op.sender,
        op.call_data.clone(),
    ));
    V1Plan {
        calls,
        deploying,
        fee,
    }
}

fn step(r: &SimCallResult) -> Step {
    Step {
        ok: r.status,
        gas: r.gas_used,
        data: r.return_data.clone(),
    }
}

fn balance_of(r: &SimCallResult) -> Result<U256> {
    if !r.status {
        return Err(sim_error("token balanceOf reverted"));
    }
    U256::abi_decode(&r.return_data).map_err(|e| sim_error(format!("balanceOf: {e}")))
}

fn decode_v1(plan: &V1Plan, op: &UserOperation, results: &[SimCallResult]) -> Result<Outcome> {
    if results.len() != plan.calls.len() {
        return Err(sim_error(format!(
            "eth_simulateV1 returned {} results for {} calls",
            results.len(),
            plan.calls.len()
        )));
    }
    let mut it = results.iter();
    let mut next = || {
        it.next()
            .ok_or_else(|| sim_error("eth_simulateV1 results ended early"))
    };
    let deploy = if plan.deploying {
        let r = next()?;
        if r.status && Address::abi_decode(&r.return_data).ok() != Some(op.sender) {
            return Err(Reject::not_allowed("factory does not deploy the sender").into());
        }
        Some(step(r))
    } else {
        None
    };
    let (fee_call, balances) = if plan.fee {
        let sender_before = balance_of(next()?)?;
        let paymaster_before = balance_of(next()?)?;
        let transfer = step(next()?);
        let sender_after = balance_of(next()?)?;
        let paymaster_after = balance_of(next()?)?;
        let balances = Balances {
            sender: (sender_before, sender_after),
            paymaster: (paymaster_before, paymaster_after),
        };
        (Some(transfer), Some(balances))
    } else {
        (None, None)
    };
    Ok(Outcome {
        deploy,
        fee_call,
        balances,
        paymaster_validation: U256::ZERO,
        exec: step(next()?),
        method: Method::SimulateV1,
    })
}

/// 备用方法的 `eth_call` 与状态覆盖。
fn entry_point_call(
    chain: &Chain,
    op: &UserOperation,
    s: &Sponsored,
    limits: Limits,
) -> Result<(TransactionRequest, StateOverride)> {
    let mut sim = op.clone();
    if limits == Limits::Estimate {
        sim.verification_gas_limit = U256::from(MAX_VERIFICATION_GAS);
        sim.call_gas_limit = U256::from(MAX_CALL_GAS);
        sim.max_fee_per_gas = U256::ZERO;
        sim.max_priority_fee_per_gas = U256::ZERO;
    }
    let key = PrivateKeySigner::random();
    let hash = s.terms.hash(&sim, chain.chain_id, s.paymaster)?;
    let signature = key
        .sign_message_sync(hash.as_slice())
        .map_err(anyhow::Error::from)?;
    sim.paymaster_data = Some(s.terms.encode(&signature.as_bytes())?);
    let p: PackedUserOperation = sim.pack()?;
    let data = ISponsorSimulations::simulateSponsoredCall {
        op: ISponsorSimulations::SimUserOperation {
            sender: p.sender,
            nonce: p.nonce,
            initCode: p.initCode,
            callData: p.callData,
            accountGasLimits: p.accountGasLimits,
            preVerificationGas: p.preVerificationGas,
            gasFees: p.gasFees,
            paymasterAndData: p.paymasterAndData,
            signature: p.signature,
        },
        token: s.token,
    }
    .abi_encode();
    let code: Bytes = SIMULATIONS_CODE
        .trim()
        .parse()
        .map_err(|e| sim_error(format!("embedded bytecode: {e}")))?;
    let mut overrides = StateOverride::default();
    overrides.insert(
        chain.entry_point,
        AccountOverride {
            code: Some(code),
            ..AccountOverride::default()
        },
    );
    let signer_slot = B256::from(PAYMASTER_SIGNER_SLOT);
    overrides.insert(
        s.paymaster,
        AccountOverride {
            state_diff: Some(std::iter::once((signer_slot, key.address().into_word())).collect()),
            ..AccountOverride::default()
        },
    );
    Ok((
        call_request(Address::ZERO, chain.entry_point, data.into()),
        overrides,
    ))
}

fn calldata_gas(b: &[u8]) -> u64 {
    b.iter().map(|&x| if x == 0 { 4 } else { 16 }).sum()
}

/// 内部调用的 gas 换算成「一笔交易」的口径，与 `eth_simulateV1` 一致。
fn as_tx_gas(gas: U256, data: &[u8]) -> Result<u64> {
    let gas = u64::try_from(gas).map_err(|_| sim_error("gas out of range"))?;
    Ok(gas + TX_BASE_GAS + calldata_gas(data))
}

/// EntryPoint 的 `FailedOp` 等错误解成可读原因。
fn revert_reason(e: &NodeError) -> String {
    let decoded = e
        .data
        .as_ref()
        .and_then(|d| IEntryPoint::IEntryPointErrors::abi_decode(d).ok());
    match decoded {
        Some(IEntryPoint::IEntryPointErrors::FailedOp(f)) => f.reason,
        Some(IEntryPoint::IEntryPointErrors::FailedOpWithRevert(f)) => {
            format!("{} {}", f.reason, f.inner)
        }
        None => e.to_string(),
    }
}

fn decode_entry_point(
    op: &UserOperation,
    r: std::result::Result<Bytes, NodeError>,
) -> Result<Outcome> {
    let out = match r {
        Ok(out) => out,
        // 有 revert 数据：这笔操作在 EntryPoint 里失败（AA13 部署失败、AA31 押金不足、AA33 收费失败……）。
        Err(e) if e.data.is_some() => {
            return Err(Reject::new(ErrorCode::SimulationFailed, revert_reason(&e)).into());
        }
        Err(e) => return Err(sim_error(format!("EntryPoint eth_call: {e}"))),
    };
    let o = ISponsorSimulations::simulateSponsoredCall::abi_decode_returns(&out)
        .map_err(|e| sim_error(format!("EntryPoint simulation returned garbage: {e}")))?;
    let deploying = op.factory.is_some_and(|f| !f.is_zero());
    let factory_data = op.factory_data.as_ref().map_or(&[][..], |d| d.as_ref());
    Ok(Outcome {
        // 部署失败时 EntryPoint 直接回滚（AA13–AA15），走到这里就是成功了。
        deploy: if deploying {
            Some(Step {
                ok: true,
                gas: as_tx_gas(o.deployGas, factory_data)?,
                data: Bytes::new(),
            })
        } else {
            None
        },
        fee_call: None,
        balances: Some(Balances {
            sender: (o.senderBalanceBefore, o.senderBalanceAfter),
            paymaster: (o.paymasterBalanceBefore, o.paymasterBalanceAfter),
        }),
        paymaster_validation: o.paymasterValidationData,
        exec: Step {
            ok: o.executed,
            gas: as_tx_gas(o.executionGas, &op.call_data)?,
            data: o.executionResult,
        },
        method: Method::EntryPoint,
    })
}

/// 节点返回的原始结果。
enum Raw {
    V1(std::result::Result<Vec<SimCallResult>, NodeError>),
    EntryPoint(std::result::Result<Bytes, NodeError>),
}

pub async fn simulate(
    chain: &Chain,
    router: &Router,
    op: &UserOperation,
    s: &Sponsored,
    limits: Limits,
) -> Result<Simulation> {
    if op.paymaster != Some(s.paymaster) {
        return Err(sim_error("operation is not sponsored by our paymaster"));
    }
    let deployed = chain.code_len(op.sender).await? > 0;
    let deploying = op.factory.is_some_and(|f| !f.is_zero());
    match (deploying, deployed) {
        (true, true) => {
            return Err(Reject::invalid("account already deployed: drop factory").into());
        }
        (false, false) => return Err(Reject::invalid("account not deployed and no factory").into()),
        _ => {}
    }
    let plan = v1_plan(chain, op, s, deploying);
    let (tx, overrides) = entry_point_call(chain, op, s, limits)?;

    let raw = chain
        .rpc
        .call_on("simulate", |i, p| {
            let calls = plan.calls.clone();
            let (tx, overrides) = (tx.clone(), overrides.clone());
            async move {
                if router.try_v1(i) {
                    match chain::simulate_v1(&p, calls).await? {
                        Err(e) if v1_unavailable(&e) => {
                            router.mark_v1_unavailable(i);
                            tracing::warn!(
                                endpoint = chain.rpc.endpoint_name(i),
                                error = %e,
                                cooldown_secs = router.cooldown.as_secs(),
                                "eth_simulateV1 unavailable, switching this endpoint to EntryPoint simulation"
                            );
                        }
                        r => return Ok(Raw::V1(r)),
                    }
                }
                Ok(Raw::EntryPoint(
                    chain::call_with_overrides(&p, tx, overrides).await?,
                ))
            }
        })
        .await?;
    let method = match raw {
        Raw::V1(_) => Method::SimulateV1,
        Raw::EntryPoint(_) => Method::EntryPoint,
    };
    router.record(method);
    tracing::debug!(sender = %op.sender, method = method.as_str(), "simulated");
    let outcome = match raw {
        Raw::V1(Ok(results)) => decode_v1(&plan, op, &results)?,
        Raw::V1(Err(e)) => return Err(sim_error(format!("eth_simulateV1: {e}"))),
        Raw::EntryPoint(r) => decode_entry_point(op, r)?,
    };
    check(&outcome, op, s.terms.fee)
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

    // ── 模拟方法选择：本机假 RPC ──

    use std::sync::Arc;

    use alloy::sol_types::SolError;
    use forum_chain::config::Endpoint;
    use forum_chain::rpc::FailoverRpc;
    use serde_json::{Value, json};

    use crate::jsonrpc::Error;

    const PAYMASTER: Address = Address::repeat_byte(0x44);
    const TOKEN: Address = Address::repeat_byte(0x55);
    const ENTRY_POINT: Address = Address::repeat_byte(0xe0);
    const SENDER: Address = Address::repeat_byte(0xaa);
    const FEE: u128 = 50_000;

    /// 假节点对 `eth_simulateV1` 与 `eth_call` 的回答。
    #[derive(Clone)]
    struct Script {
        v1: Value,
        call: Value,
    }

    struct FakeNode {
        url: String,
        /// 收到的 (方法, 参数)。
        seen: Arc<Mutex<Vec<(String, Value)>>>,
    }

    impl FakeNode {
        fn methods(&self) -> Vec<String> {
            self.seen
                .lock()
                .unwrap()
                .iter()
                .map(|(m, _)| m.clone())
                .collect()
        }

        fn last(&self, method: &str) -> Value {
            let seen = self.seen.lock().unwrap();
            seen.iter()
                .rev()
                .find(|(m, _)| m == method)
                .unwrap()
                .1
                .clone()
        }
    }

    async fn fake_node(script: Script) -> FakeNode {
        let seen: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
        let log = seen.clone();
        let app = axum::Router::new().route(
            "/",
            axum::routing::post(move |axum::Json(req): axum::Json<Value>| {
                let (log, script) = (log.clone(), script.clone());
                async move {
                    let method = req["method"].as_str().unwrap().to_owned();
                    log.lock()
                        .unwrap()
                        .push((method.clone(), req["params"].clone()));
                    let answer = match method.as_str() {
                        "eth_getCode" => json!({"result": "0x6001"}),
                        "eth_simulateV1" => script.v1,
                        "eth_call" => script.call,
                        other => panic!("unexpected {other}"),
                    };
                    let mut body = json!({"jsonrpc": "2.0", "id": req["id"]});
                    body.as_object_mut()
                        .unwrap()
                        .extend(answer.as_object().unwrap().clone());
                    axum::Json(body)
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        FakeNode { url, seen }
    }

    fn chain(node: &FakeNode) -> Chain {
        let e = Endpoint {
            http: node.url.clone(),
            ws: None,
        };
        Chain {
            rpc: FailoverRpc::new(&[&e], Duration::from_secs(300)).unwrap(),
            chain_id: 421_614,
            entry_point: ENTRY_POINT,
        }
    }

    fn nitro_crash() -> Value {
        json!({"error": {"code": -32603, "message": "method handler crashed"}})
    }

    fn sponsored_op() -> UserOperation {
        UserOperation {
            sender: SENDER,
            call_data: vec![0xab; 40].into(),
            paymaster: Some(PAYMASTER),
            paymaster_verification_gas_limit: Some(U256::from(PM_VERIFICATION_GAS)),
            paymaster_post_op_gas_limit: Some(U256::from(PM_POST_OP_GAS)),
            ..op()
        }
    }

    fn sponsored() -> Sponsored {
        Sponsored {
            paymaster: PAYMASTER,
            token: TOKEN,
            terms: SponsorTerms {
                valid_until: 2_000_000_120,
                valid_after: 2_000_000_000,
                fee: FEE,
            },
        }
    }

    fn outcome() -> ISponsorSimulations::Outcome {
        ISponsorSimulations::Outcome {
            deployGas: U256::ZERO,
            // validUntil / validAfter 在高位，签名有效（低 160 位为 0）。
            paymasterValidationData: U256::from(2_000_000_120u64) << 160,
            senderBalanceBefore: U256::from(1_000_000),
            senderBalanceAfter: U256::from(1_000_000 - FEE),
            paymasterBalanceBefore: U256::from(7),
            paymasterBalanceAfter: U256::from(7 + FEE),
            executed: true,
            executionGas: U256::from(30_000),
            executionResult: Bytes::new(),
        }
    }

    fn returns(o: ISponsorSimulations::Outcome) -> Value {
        let data = ISponsorSimulations::simulateSponsoredCall::abi_encode_returns(&o);
        json!({"result": Bytes::from(data)})
    }

    async fn run(node: &FakeNode, router: &Router) -> Result<Simulation> {
        simulate(
            &chain(node),
            router,
            &sponsored_op(),
            &sponsored(),
            Limits::Signed,
        )
        .await
    }

    fn reject_code(r: Result<Simulation>) -> ErrorCode {
        match r {
            Err(Error::Reject(r)) => r.code,
            other => panic!("expected a rejection, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn nitro_crash_switches_the_endpoint_to_entry_point_simulation() {
        let node = fake_node(Script {
            v1: nitro_crash(),
            call: returns(outcome()),
        })
        .await;
        let router = Router::default();
        let s = run(&node, &router).await.unwrap();
        assert_eq!(s.method, Method::EntryPoint);
        assert_eq!(
            s.exec_gas,
            30_000 + TX_BASE_GAS + 40 * 16,
            "same accounting as eth_simulateV1"
        );
        assert_eq!(
            node.methods(),
            ["eth_getCode", "eth_simulateV1", "eth_call"]
        );

        // 状态覆盖：EntryPoint 换成模拟合约，Paymaster 的 signer 槽换成一次性密钥。
        let params = node.last("eth_call");
        assert_eq!(params[0]["to"], json!(ENTRY_POINT));
        let overrides = &params[2];
        assert_eq!(
            overrides[ENTRY_POINT.to_string().to_lowercase()]["code"],
            json!(SIMULATIONS_CODE.trim())
        );
        let diff = overrides[PAYMASTER.to_string().to_lowercase()]["stateDiff"]
            .as_object()
            .unwrap();
        let (slot, signer) = diff.iter().next().unwrap();
        assert_eq!(
            slot.parse::<B256>().unwrap(),
            B256::from(PAYMASTER_SIGNER_SLOT)
        );
        assert_ne!(
            signer.as_str().unwrap().parse::<B256>().unwrap(),
            B256::ZERO
        );

        // 冷却期内同一节点不再试 eth_simulateV1。
        let s = run(&node, &router).await.unwrap();
        assert_eq!(s.method, Method::EntryPoint);
        assert_eq!(
            node.methods()
                .iter()
                .filter(|m| *m == "eth_simulateV1")
                .count(),
            1
        );
        assert_eq!(
            router.counts(),
            MethodCounts {
                simulate_v1: 0,
                entry_point: 2
            }
        );
    }

    #[tokio::test]
    async fn cooldown_expiry_retries_simulate_v1() {
        let node = fake_node(Script {
            v1: nitro_crash(),
            call: returns(outcome()),
        })
        .await;
        let router = Router::new(Duration::ZERO);
        run(&node, &router).await.unwrap();
        run(&node, &router).await.unwrap();
        assert_eq!(
            node.methods()
                .iter()
                .filter(|m| *m == "eth_simulateV1")
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn fallback_failures_refuse_to_sign() {
        let mut not_executed = outcome();
        not_executed.executed = false;
        let mut fee_not_pulled = outcome();
        fee_not_pulled.paymasterBalanceAfter = fee_not_pulled.paymasterBalanceBefore;
        let mut sender_overcharged = outcome();
        sender_overcharged.senderBalanceAfter -= U256::from(1);
        let failed_op = IEntryPoint::FailedOp {
            opIndex: U256::ZERO,
            reason: "AA31 paymaster deposit too low".into(),
        };
        let reverted = json!({"error": {
            "code": 3, "message": "execution reverted",
            "data": Bytes::from(failed_op.abi_encode()),
        }});
        for call in [
            returns(not_executed),
            returns(fee_not_pulled),
            returns(sender_overcharged),
            reverted,
        ] {
            let node = fake_node(Script {
                v1: nitro_crash(),
                call,
            })
            .await;
            assert_eq!(
                reject_code(run(&node, &Router::default()).await),
                ErrorCode::SimulationFailed
            );
        }
    }

    #[tokio::test]
    async fn ambiguous_answers_are_internal_errors() {
        let mut bad_signer = outcome();
        bad_signer.paymasterValidationData |= U256::from(1);
        for (v1, call) in [
            // 不是「方法不可用」的错误：不换方法，直接拒签。
            (
                json!({"error": {"code": -32000, "message": "header not found"}}),
                returns(outcome()),
            ),
            (nitro_crash(), returns(bad_signer)),
            (nitro_crash(), json!({"result": "0x1234"})),
            (
                nitro_crash(),
                json!({"error": {"code": -32000, "message": "state override not supported"}}),
            ),
        ] {
            let node = fake_node(Script { v1, call }).await;
            let r = run(&node, &Router::default()).await;
            assert!(matches!(r, Err(Error::Internal(_))), "{r:?}");
        }
    }

    #[tokio::test]
    async fn simulate_v1_checks_token_movements() {
        let ok = |v: u64| SimCallResult {
            return_data: U256::from(v).abi_encode().into(),
            logs: Vec::new(),
            gas_used: 21_000,
            status: true,
            error: None,
            max_used_gas: None,
        };
        let calls = |sender_after: u64| {
            let block = alloy::rpc::types::simulate::SimulatedBlock {
                inner: alloy::rpc::types::Block::<alloy::rpc::types::Transaction>::default(),
                calls: vec![
                    ok(1_000_000),
                    ok(7),
                    ok(1),
                    ok(sender_after),
                    ok(7 + 50_000),
                    ok(0),
                ],
            };
            json!({ "result": [block] })
        };
        let node = fake_node(Script {
            v1: calls(1_000_000 - 50_000),
            call: json!({"error": {"code": -32000, "message": "unused"}}),
        })
        .await;
        let router = Router::default();
        assert_eq!(
            run(&node, &router).await.unwrap().method,
            Method::SimulateV1
        );
        assert!(!node.methods().contains(&"eth_call".to_owned()));

        let node = fake_node(Script {
            v1: calls(1_000_000 - 50_001),
            call: json!({"error": {"code": -32000, "message": "unused"}}),
        })
        .await;
        assert_eq!(
            reject_code(run(&node, &router).await),
            ErrorCode::SimulationFailed
        );
    }

    #[test]
    fn unavailable_simulate_v1_errors() {
        let e = |code, message: &str| NodeError {
            code,
            message: message.into(),
            data: None,
        };
        assert!(v1_unavailable(&e(-32603, "method handler crashed")));
        assert!(v1_unavailable(&e(
            -32601,
            "the method eth_simulateV1 does not exist/is not available"
        )));
        assert!(v1_unavailable(&e(
            -32600,
            "Unsupported method: eth_simulateV1"
        )));
        assert!(!v1_unavailable(&e(-32603, "internal error")));
        assert!(!v1_unavailable(&e(-32000, "header not found")));
        assert!(!v1_unavailable(&e(3, "execution reverted")));
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
