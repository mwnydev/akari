//! 代付白名单与费用档位（plan 第 5 节「gas 规则」「次数上限」，ADR 0004）。纯函数，不碰网络和数据库。
//!
//! - 只认 USDT、Escrow、StreamEscrow 与账户自身（签名者变更）；Paymaster 合约和其他任何地址一律拒绝。
//! - USDT 只能 `approve` 给担保合约，或 `transfer` 提回交易所：收款方必须是用户已生效的提币地址
//!   （plan 第 5 节「提币地址冷却」；冷却中或已撤销的地址不代付）。
//! - 付款前的操作（下单、提回、超时取回退款、撤销报价、改签名者……）收代付费；付款后的操作免费，
//!   由担保合约结算时扣固定 gas 费。免费额度用完的操作：标记发货、开始、领取直接拒绝，延长确认期与和解改收代付费。

use std::collections::HashMap;

use alloy::primitives::{Address, U256};
use alloy::sol_types::SolCall;
use forum_chain::bindings::Escrow;
use forum_chain::bindings::stream::StreamEscrow;
pub use forum_chain::order::Market;
use forum_core::ErrorCode;

use crate::jsonrpc::Reject;
use crate::kernel::{Call, IKernel};

alloy::sol! {
    interface IERC20 {
        function approve(address spender, uint256 amount) external returns (bool);
        function transfer(address to, uint256 amount) external returns (bool);
    }
}

/// 一笔操作最多几次调用（下单是 approve + createAndFund 两次）。
pub const MAX_CALLS: usize = 4;
/// 「领取每天 1 次」的窗口。
pub const DAY_SECS: i64 = 86_400;

/// 白名单涉及的合约地址。
#[derive(Debug, Clone, Copy)]
pub struct Contracts {
    pub token: Address,
    pub escrow: Address,
    pub stream: Address,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    CreateAndFund,
    CreateFromAuction,
    CancelOffer,
    Withdraw,
    DepositBond,
    WithdrawBond,
    MarkDelivered,
    Start,
    Release,
    Claim,
    Finalize,
    Refund,
    ClaimTimeout,
    ExtendConfirm,
    ProposeEnd,
    AcceptEnd,
    Dispute,
    DisputeWithBond,
}

impl Method {
    /// 合约函数名；也是 `sponsored_calls.method` 的值。
    pub fn name(self) -> &'static str {
        match self {
            Self::CreateAndFund => "createAndFund",
            Self::CreateFromAuction => "createFromAuction",
            Self::CancelOffer => "cancelOffer",
            Self::Withdraw => "withdraw",
            Self::DepositBond => "depositBond",
            Self::WithdrawBond => "withdrawBond",
            Self::MarkDelivered => "markDelivered",
            Self::Start => "start",
            Self::Release => "release",
            Self::Claim => "claim",
            Self::Finalize => "finalize",
            Self::Refund => "refund",
            Self::ClaimTimeout => "claimTimeout",
            Self::ExtendConfirm => "extendConfirm",
            Self::ProposeEnd => "proposeEnd",
            Self::AcceptEnd => "acceptEnd",
            Self::Dispute => "dispute",
            Self::DisputeWithBond => "disputeWithBond",
        }
    }
}

/// 解码并通过白名单的一次调用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// USDT 授权给担保合约（下单、交保证金前）。
    Approve,
    /// USDT 转出（提回交易所）。
    Transfer { to: Address },
    /// 账户自身安装 / 卸载验证器（改签名者）。
    SignerChange,
    /// 不针对某个订单的担保合约方法（下单、撤销报价、提取、保证金）。
    Market { market: Market, method: Method },
    /// 针对某个订单的担保合约方法。
    Order {
        contract: Address,
        market: Market,
        method: Method,
        order_id: U256,
    },
}

/// 不在免费额度内时怎么办。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Over {
    Reject,
    Fee,
}

/// 订单方法的免费额度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quota {
    pub limit: i64,
    /// 按调用方（买家、卖家各自）计数。
    pub per_side: bool,
    /// 只数最近 [`DAY_SECS`] 秒。
    pub per_day: bool,
    pub over: Over,
}

/// 费用档位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Fee,
    Free(Option<Quota>),
    /// Escrow `claimTimeout`：已发货是确认期满放款（付款后，免费），未发货是超时退款（付款前，收费）。
    FreeIfDelivered,
}

const fn quota(limit: i64, per_side: bool, per_day: bool, over: Over) -> Tier {
    Tier::Free(Some(Quota {
        limit,
        per_side,
        per_day,
        over,
    }))
}

pub fn tier(market: Market, method: Method) -> Tier {
    use Method::*;
    match method {
        CreateAndFund | CreateFromAuction | CancelOffer | Withdraw | DepositBond | WithdrawBond
        | Refund => Tier::Fee,
        ClaimTimeout if market == Market::Escrow => Tier::FreeIfDelivered,
        ClaimTimeout => Tier::Fee,
        MarkDelivered | Start => quota(1, false, false, Over::Reject),
        Claim => quota(1, false, true, Over::Reject),
        ExtendConfirm => quota(1, false, false, Over::Fee),
        ProposeEnd | AcceptEnd => quota(1, true, false, Over::Fee),
        Release | Finalize | Dispute | DisputeWithBond => Tier::Free(None),
    }
}

fn not_allowed(i: usize, what: &str) -> Reject {
    Reject::not_allowed(format!("call {i}: {what}"))
}

fn order<C: SolCall>(data: &[u8], i: usize, id: impl Fn(&C) -> U256) -> Result<U256, Reject> {
    C::abi_decode(data)
        .map(|c| id(&c))
        .map_err(|_| not_allowed(i, "malformed arguments"))
}

fn escrow_method(data: &[u8], i: usize) -> Result<(Method, Option<U256>), Reject> {
    use Escrow::*;
    let sel: [u8; 4] = data
        .get(..4)
        .and_then(|s| s.try_into().ok())
        .ok_or_else(|| not_allowed(i, "missing selector"))?;
    let decoded = match sel {
        createAndFundCall::SELECTOR => (Method::CreateAndFund, None),
        createFromAuctionCall::SELECTOR => (Method::CreateFromAuction, None),
        cancelOfferCall::SELECTOR => (Method::CancelOffer, None),
        withdrawCall::SELECTOR => (Method::Withdraw, None),
        depositBondCall::SELECTOR => (Method::DepositBond, None),
        withdrawBondCall::SELECTOR => (Method::WithdrawBond, None),
        markDeliveredCall::SELECTOR => (
            Method::MarkDelivered,
            Some(order::<markDeliveredCall>(data, i, |c| c.orderId)?),
        ),
        releaseCall::SELECTOR => (
            Method::Release,
            Some(order::<releaseCall>(data, i, |c| c.orderId)?),
        ),
        refundCall::SELECTOR => (
            Method::Refund,
            Some(order::<refundCall>(data, i, |c| c.orderId)?),
        ),
        claimTimeoutCall::SELECTOR => (
            Method::ClaimTimeout,
            Some(order::<claimTimeoutCall>(data, i, |c| c.orderId)?),
        ),
        extendConfirmCall::SELECTOR => (
            Method::ExtendConfirm,
            Some(order::<extendConfirmCall>(data, i, |c| c.orderId)?),
        ),
        proposeEndCall::SELECTOR => (
            Method::ProposeEnd,
            Some(order::<proposeEndCall>(data, i, |c| c.orderId)?),
        ),
        acceptEndCall::SELECTOR => (
            Method::AcceptEnd,
            Some(order::<acceptEndCall>(data, i, |c| c.orderId)?),
        ),
        disputeCall::SELECTOR => (
            Method::Dispute,
            Some(order::<disputeCall>(data, i, |c| c.orderId)?),
        ),
        disputeWithBondCall::SELECTOR => (
            Method::DisputeWithBond,
            Some(order::<disputeWithBondCall>(data, i, |c| c.orderId)?),
        ),
        _ => return Err(not_allowed(i, "escrow method not sponsored")),
    };
    Ok(decoded)
}

fn stream_method(data: &[u8], i: usize) -> Result<(Method, Option<U256>), Reject> {
    use StreamEscrow::*;
    let sel: [u8; 4] = data
        .get(..4)
        .and_then(|s| s.try_into().ok())
        .ok_or_else(|| not_allowed(i, "missing selector"))?;
    let decoded = match sel {
        createAndFundCall::SELECTOR => (Method::CreateAndFund, None),
        cancelOfferCall::SELECTOR => (Method::CancelOffer, None),
        withdrawCall::SELECTOR => (Method::Withdraw, None),
        startCall::SELECTOR => (
            Method::Start,
            Some(order::<startCall>(data, i, |c| c.orderId)?),
        ),
        claimCall::SELECTOR => (
            Method::Claim,
            Some(order::<claimCall>(data, i, |c| c.orderId)?),
        ),
        finalizeCall::SELECTOR => (
            Method::Finalize,
            Some(order::<finalizeCall>(data, i, |c| c.orderId)?),
        ),
        refundCall::SELECTOR => (
            Method::Refund,
            Some(order::<refundCall>(data, i, |c| c.orderId)?),
        ),
        claimTimeoutCall::SELECTOR => (
            Method::ClaimTimeout,
            Some(order::<claimTimeoutCall>(data, i, |c| c.orderId)?),
        ),
        proposeEndCall::SELECTOR => (
            Method::ProposeEnd,
            Some(order::<proposeEndCall>(data, i, |c| c.orderId)?),
        ),
        acceptEndCall::SELECTOR => (
            Method::AcceptEnd,
            Some(order::<acceptEndCall>(data, i, |c| c.orderId)?),
        ),
        disputeCall::SELECTOR => (
            Method::Dispute,
            Some(order::<disputeCall>(data, i, |c| c.orderId)?),
        ),
        disputeWithBondCall::SELECTOR => (
            Method::DisputeWithBond,
            Some(order::<disputeWithBondCall>(data, i, |c| c.orderId)?),
        ),
        _ => return Err(not_allowed(i, "stream escrow method not sponsored")),
    };
    Ok(decoded)
}

fn market_action(contract: Address, market: Market, decoded: (Method, Option<U256>)) -> Action {
    match decoded {
        (method, Some(order_id)) => Action::Order {
            contract,
            market,
            method,
            order_id,
        },
        (method, None) => Action::Market { market, method },
    }
}

/// 逐笔检查白名单。`sender` 是发起操作的账户（签名者变更是账户调用自己）；`withdraw_to` 是用户已生效的提币地址。
pub fn classify(
    calls: &[Call],
    sender: Address,
    c: &Contracts,
    withdraw_to: &[Address],
) -> Result<Vec<Action>, Reject> {
    if calls.is_empty() || calls.len() > MAX_CALLS {
        return Err(Reject::not_allowed(format!(
            "operation must contain 1..={MAX_CALLS} calls"
        )));
    }
    calls
        .iter()
        .enumerate()
        .map(|(i, call)| {
            if !call.value.is_zero() {
                return Err(not_allowed(i, "ETH value not allowed"));
            }
            let data = call.data.as_ref();
            if call.target == c.token {
                if let Ok(a) = IERC20::approveCall::abi_decode(data) {
                    let to_market = a.spender == c.escrow || a.spender == c.stream;
                    return if to_market {
                        Ok(Action::Approve)
                    } else {
                        Err(not_allowed(i, "approve spender not allowed"))
                    };
                }
                if let Ok(t) = IERC20::transferCall::abi_decode(data) {
                    return if withdraw_to.contains(&t.to) {
                        Ok(Action::Transfer { to: t.to })
                    } else {
                        Err(Reject::new(
                            ErrorCode::WithdrawAddressNotActive,
                            format!(
                                "call {i}: transfer recipient is not an active withdraw address"
                            ),
                        ))
                    };
                }
                return Err(not_allowed(i, "token method not sponsored"));
            }
            if call.target == c.escrow {
                return Ok(market_action(
                    call.target,
                    Market::Escrow,
                    escrow_method(data, i)?,
                ));
            }
            if call.target == c.stream {
                return Ok(market_action(
                    call.target,
                    Market::Stream,
                    stream_method(data, i)?,
                ));
            }
            if call.target == sender {
                let sel = data.get(..4);
                let is_signer_change = sel == Some(&IKernel::installValidationsCall::SELECTOR[..])
                    || sel == Some(&IKernel::uninstallValidationCall::SELECTOR[..]);
                return if is_signer_change {
                    Ok(Action::SignerChange)
                } else {
                    Err(not_allowed(i, "account method not sponsored"))
                };
            }
            Err(not_allowed(i, "target not allowed"))
        })
        .collect()
}

/// 定价需要的外部事实，与 `classify` 的结果一一对应。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Facts {
    /// 这一项额度此前已用掉的次数（数据库）。
    pub used: i64,
    /// Escrow `claimTimeout`：订单已发货（链上 latest）。
    pub delivered: bool,
}

/// 记入 `sponsored_calls` 的订单调用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderCall {
    pub contract: Address,
    pub order_id: U256,
    pub method: Method,
    pub free: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Priced {
    /// 这笔操作是否收代付费（一笔操作最多收一次）。
    pub fee_due: bool,
    pub calls: Vec<OrderCall>,
}

/// 按档位与额度定价。
pub fn price(actions: &[Action], facts: &[Facts]) -> Result<Priced, Reject> {
    debug_assert_eq!(actions.len(), facts.len());
    let mut fee_due = false;
    let mut calls = Vec::new();
    // 同一笔操作里重复调用同一订单的同一方法，也要占额度。
    let mut in_batch: HashMap<(Address, U256, Method), i64> = HashMap::new();
    for (action, f) in actions.iter().zip(facts) {
        let Action::Order {
            contract,
            market,
            method,
            order_id,
        } = action
        else {
            fee_due = true;
            continue;
        };
        let free = match tier(*market, *method) {
            Tier::Fee => false,
            Tier::FreeIfDelivered => f.delivered,
            Tier::Free(None) => true,
            Tier::Free(Some(q)) => {
                let seen = in_batch.entry((*contract, *order_id, *method)).or_insert(0);
                let used = f.used + *seen;
                *seen += 1;
                if used < q.limit {
                    true
                } else if q.over == Over::Fee {
                    false
                } else {
                    return Err(Reject::new(
                        ErrorCode::SponsorQuotaExceeded,
                        format!("{} quota used for order {order_id}", method.name()),
                    ));
                }
            }
        };
        fee_due |= !free;
        calls.push(OrderCall {
            contract: *contract,
            order_id: *order_id,
            method: *method,
            free,
        });
    }
    Ok(Priced { fee_due, calls })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use alloy::primitives::Bytes;
    use alloy::sol_types::SolCall;

    use super::*;

    const TOKEN: Address = Address::repeat_byte(0x01);
    const ESCROW: Address = Address::repeat_byte(0x02);
    const STREAM: Address = Address::repeat_byte(0x03);
    const PAYMASTER: Address = Address::repeat_byte(0x04);
    const SENDER: Address = Address::repeat_byte(0x05);

    fn contracts() -> Contracts {
        Contracts {
            token: TOKEN,
            escrow: ESCROW,
            stream: STREAM,
        }
    }

    fn call(target: Address, data: impl SolCall) -> Call {
        Call {
            target,
            value: U256::ZERO,
            data: data.abi_encode().into(),
        }
    }

    fn id(n: u64) -> U256 {
        U256::from(n)
    }

    const WITHDRAW_TO: Address = Address::repeat_byte(0x99);

    fn classify1(c: Call) -> Result<Action, Reject> {
        classify(&[c], SENDER, &contracts(), &[WITHDRAW_TO]).map(|mut v| v.remove(0))
    }

    #[test]
    fn approve_only_to_markets() {
        let ok = call(
            TOKEN,
            IERC20::approveCall {
                spender: ESCROW,
                amount: id(5),
            },
        );
        assert_eq!(classify1(ok).unwrap(), Action::Approve);
        let ok = call(
            TOKEN,
            IERC20::approveCall {
                spender: STREAM,
                amount: id(5),
            },
        );
        assert_eq!(classify1(ok).unwrap(), Action::Approve);
        let bad = call(
            TOKEN,
            IERC20::approveCall {
                spender: PAYMASTER,
                amount: id(5),
            },
        );
        assert!(classify1(bad).is_err());
    }

    #[test]
    fn transfer_only_to_active_withdraw_addresses() {
        let c = call(
            TOKEN,
            IERC20::transferCall {
                to: WITHDRAW_TO,
                amount: id(5),
            },
        );
        assert_eq!(classify1(c).unwrap(), Action::Transfer { to: WITHDRAW_TO });
        // 冷却中、已撤销或从没登记过的地址都不在已生效列表里；转给自己的钱包或合约同样拒绝。
        for to in [Address::repeat_byte(0x98), SENDER, ESCROW, PAYMASTER] {
            let c = call(TOKEN, IERC20::transferCall { to, amount: id(5) });
            assert_eq!(
                classify1(c).unwrap_err().code,
                ErrorCode::WithdrawAddressNotActive,
                "{to}"
            );
        }
    }

    #[test]
    fn rejects_other_targets_value_and_selectors() {
        let to_paymaster = Call {
            target: PAYMASTER,
            value: U256::ZERO,
            data: Bytes::from_static(&[1, 2, 3, 4]),
        };
        assert!(classify1(to_paymaster).is_err());
        let mut with_value = call(ESCROW, Escrow::releaseCall { orderId: id(1) });
        with_value.value = id(1);
        assert!(classify1(with_value).is_err());
        let arbiter_only = call(
            ESCROW,
            Escrow::resolveCall {
                orderId: id(1),
                toBuyerBps: id(0),
                bondToBuyerBps: id(0),
            },
        );
        assert!(classify1(arbiter_only).is_err());
        let start_on_escrow = call(ESCROW, StreamEscrow::startCall { orderId: id(1) });
        assert!(classify1(start_on_escrow).is_err());
        let truncated = Call {
            target: ESCROW,
            value: U256::ZERO,
            data: Escrow::markDeliveredCall::SELECTOR.to_vec().into(),
        };
        assert!(classify1(truncated).is_err());
        let self_execute = Call {
            target: SENDER,
            value: U256::ZERO,
            data: Bytes::from_static(&[0xe9, 0xae, 0x5c, 0x53]),
        };
        assert!(classify1(self_execute).is_err());
        assert!(classify(&[], SENDER, &contracts(), &[]).is_err());
        let many = vec![call(ESCROW, Escrow::releaseCall { orderId: id(1) }); MAX_CALLS + 1];
        assert!(classify(&many, SENDER, &contracts(), &[]).is_err());
    }

    #[test]
    fn signer_change_is_a_self_call() {
        let c = Call {
            target: SENDER,
            value: U256::ZERO,
            data: IKernel::uninstallValidationCall::SELECTOR.to_vec().into(),
        };
        assert_eq!(classify1(c).unwrap(), Action::SignerChange);
    }

    #[test]
    fn decodes_order_ids() {
        let c = call(ESCROW, Escrow::markDeliveredCall { orderId: id(7) });
        assert_eq!(
            classify1(c).unwrap(),
            Action::Order {
                contract: ESCROW,
                market: Market::Escrow,
                method: Method::MarkDelivered,
                order_id: id(7)
            }
        );
        let c = call(STREAM, StreamEscrow::claimCall { orderId: id(9) });
        assert!(matches!(
            classify1(c).unwrap(),
            Action::Order {
                method: Method::Claim,
                market: Market::Stream,
                ..
            }
        ));
        let c = call(ESCROW, Escrow::cancelOfferCall { nonce: id(3) });
        assert_eq!(
            classify1(c).unwrap(),
            Action::Market {
                market: Market::Escrow,
                method: Method::CancelOffer
            }
        );
    }

    fn order(market: Market, method: Method, n: u64) -> Action {
        Action::Order {
            contract: if market == Market::Escrow {
                ESCROW
            } else {
                STREAM
            },
            market,
            method,
            order_id: id(n),
        }
    }

    fn price1(a: Action, f: Facts) -> Result<Priced, Reject> {
        price(&[a], &[f])
    }

    #[test]
    fn pre_payment_operations_pay_the_fee() {
        let pay = vec![
            Action::Approve,
            Action::Market {
                market: Market::Escrow,
                method: Method::CreateAndFund,
            },
        ];
        let p = price(&pay, &[Facts::default(); 2]).unwrap();
        assert!(p.fee_due);
        assert!(p.calls.is_empty());
        for a in [
            Action::Transfer { to: SENDER },
            Action::SignerChange,
            Action::Market {
                market: Market::Escrow,
                method: Method::CancelOffer,
            },
            order(Market::Escrow, Method::Refund, 1),
            order(Market::Stream, Method::ClaimTimeout, 1),
        ] {
            assert!(
                price1(a.clone(), Facts::default()).unwrap().fee_due,
                "{a:?}"
            );
        }
    }

    #[test]
    fn post_payment_operations_are_free() {
        for a in [
            order(Market::Escrow, Method::MarkDelivered, 1),
            order(Market::Escrow, Method::Release, 1),
            order(Market::Escrow, Method::Dispute, 1),
            order(Market::Escrow, Method::ProposeEnd, 1),
            order(Market::Escrow, Method::AcceptEnd, 1),
            order(Market::Escrow, Method::ExtendConfirm, 1),
            order(Market::Stream, Method::Start, 1),
            order(Market::Stream, Method::Claim, 1),
            order(Market::Stream, Method::Finalize, 1),
        ] {
            let p = price1(a.clone(), Facts::default()).unwrap();
            assert!(!p.fee_due, "{a:?}");
            assert!(p.calls[0].free);
        }
    }

    #[test]
    fn claim_timeout_depends_on_delivery() {
        let a = order(Market::Escrow, Method::ClaimTimeout, 1);
        assert!(price1(a.clone(), Facts::default()).unwrap().fee_due);
        let delivered = Facts {
            delivered: true,
            ..Facts::default()
        };
        assert!(!price1(a, delivered).unwrap().fee_due);
    }

    #[test]
    fn hard_quotas_reject_and_soft_quotas_charge() {
        let used = Facts {
            used: 1,
            ..Facts::default()
        };
        let e = price1(order(Market::Escrow, Method::MarkDelivered, 1), used).unwrap_err();
        assert_eq!(e.code, ErrorCode::SponsorQuotaExceeded);
        for m in [Method::Start, Method::Claim] {
            let e = price1(order(Market::Stream, m, 1), used).unwrap_err();
            assert_eq!(e.code, ErrorCode::SponsorQuotaExceeded);
        }
        for m in [Method::ExtendConfirm, Method::ProposeEnd, Method::AcceptEnd] {
            let p = price1(order(Market::Escrow, m, 1), used).unwrap();
            assert!(p.fee_due, "{m:?}");
            assert!(!p.calls[0].free);
        }
    }

    #[test]
    fn duplicates_in_one_operation_share_the_quota() {
        let a = order(Market::Escrow, Method::ExtendConfirm, 1);
        let p = price(&[a.clone(), a], &[Facts::default(); 2]).unwrap();
        assert!(p.fee_due);
        assert_eq!(
            p.calls.iter().map(|c| c.free).collect::<Vec<_>>(),
            [true, false]
        );
        let a = order(Market::Escrow, Method::MarkDelivered, 1);
        assert!(price(&[a.clone(), a], &[Facts::default(); 2]).is_err());
        let b = order(Market::Escrow, Method::MarkDelivered, 2);
        let p = price(
            &[order(Market::Escrow, Method::MarkDelivered, 1), b],
            &[Facts::default(); 2],
        );
        assert!(!p.unwrap().fee_due, "different orders have separate quotas");
    }

    #[test]
    fn quota_rules_match_plan() {
        let q = |m| match tier(Market::Escrow, m) {
            Tier::Free(Some(q)) => q,
            t => panic!("{m:?}: {t:?}"),
        };
        assert!(q(Method::ProposeEnd).per_side);
        assert!(!q(Method::ExtendConfirm).per_side);
        assert!(matches!(tier(Market::Stream, Method::Claim), Tier::Free(Some(q)) if q.per_day));
    }
}
