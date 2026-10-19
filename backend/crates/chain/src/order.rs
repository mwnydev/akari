//! 订单状态 = 该订单全部事件按 (区块号, 日志序号) 折叠的结果。
//!
//! 折叠是纯函数：区块回滚时删掉分叉上的事件、重放主链事件，再折叠一次就得到正确状态，不需要「反向操作」。

use alloy::primitives::{Address, B256, Bytes, U256};
use alloy::sol_types::SolEventInterface;

use crate::ChainError;
use crate::bindings::Escrow::EscrowEvents;
use crate::snapshot::OrderSnapshot;

/// 链上确认级别（plan 第 2 节「链上状态分级确认」）；顺序即确认程度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// 最新区块：界面、即时通知、代付校验。
    Latest,
    /// RPC `safe` 标记：数字商品自动发卡密。
    Safe,
    /// RPC `finalized` 标记：财务、信誉、经验与薪资。
    Finalized,
}

impl Level {
    /// 与数据库枚举 `chain_level` 的取值一致。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Latest => "latest",
            Self::Safe => "safe",
            Self::Finalized => "finalized",
        }
    }

    pub fn parse(s: &str) -> Result<Self, ChainError> {
        match s {
            "latest" => Ok(Self::Latest),
            "safe" => Ok(Self::Safe),
            "finalized" => Ok(Self::Finalized),
            _ => Err(ChainError::Decode(format!("unknown level {s}"))),
        }
    }
}

/// 与 `Escrow.State` 一致；`None` 也用来表示「事件已被回滚、订单暂不存在」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderState {
    None = 0,
    Funded = 1,
    Disputed = 2,
    Released = 3,
    Refunded = 4,
    Settled = 5,
}

impl OrderState {
    pub fn from_u8(v: u8) -> Result<Self, ChainError> {
        Ok(match v {
            0 => Self::None,
            1 => Self::Funded,
            2 => Self::Disputed,
            3 => Self::Released,
            4 => Self::Refunded,
            5 => Self::Settled,
            _ => return Err(ChainError::Decode(format!("unknown order state {v}"))),
        })
    }

    pub fn code(self) -> i16 {
        self as i16
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Funded => "funded",
            Self::Disputed => "disputed",
            Self::Released => "released",
            Self::Refunded => "refunded",
            Self::Settled => "settled",
        }
    }
}

/// 影响订单状态的事件（`sol!` 类型解码后的精简形式）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderEvent {
    Created {
        buyer: Address,
        seller: Address,
        token: Address,
        amount: U256,
        offer_hash: B256,
        listing_hash: B256,
        instant: bool,
    },
    AuctionSettled {
        auction_id: U256,
    },
    Delivered {
        confirm_deadline: u64,
    },
    ConfirmExtended {
        confirm_deadline: u64,
    },
    DeadlineExtended {
        deadline: u64,
    },
    EndProposed {
        proposer: Address,
        to_buyer: U256,
        expiry: u64,
    },
    Disputed {
        by: Address,
        fee: U256,
        from_bond: bool,
    },
    Resolved {
        to_buyer_bps: U256,
        bond_to_buyer_bps: U256,
    },
    Settled {
        state: OrderState,
        to_buyer: U256,
        to_seller: U256,
        to_fee_recipient: U256,
    },
}

fn order_id(id: U256) -> Result<u64, ChainError> {
    u64::try_from(id).map_err(|_| ChainError::Decode(format!("order id {id} exceeds u64")))
}

/// 解码订单类事件；其他事件（押金池、配置、提现等）返回 `None`。
pub fn decode(topics: &[B256], data: &[u8]) -> Result<Option<(u64, OrderEvent)>, ChainError> {
    let Some(topic0) = topics.first() else {
        return Ok(None);
    };
    if !EscrowEvents::SELECTORS.contains(&topic0.0) {
        return Ok(None);
    }
    let ev = EscrowEvents::decode_raw_log(topics, data)
        .map_err(|e| ChainError::Decode(format!("escrow event: {e}")))?;
    Ok(Some(match ev {
        EscrowEvents::OrderCreated(e) => (
            order_id(e.orderId)?,
            OrderEvent::Created {
                buyer: e.buyer,
                seller: e.seller,
                token: e.token,
                amount: e.amount,
                offer_hash: e.offerHash,
                listing_hash: e.listingHash,
                instant: e.instant,
            },
        ),
        EscrowEvents::AuctionSettled(e) => (
            order_id(e.orderId)?,
            OrderEvent::AuctionSettled {
                auction_id: e.auctionId,
            },
        ),
        EscrowEvents::Delivered(e) => (
            order_id(e.orderId)?,
            OrderEvent::Delivered {
                confirm_deadline: e.confirmDeadline,
            },
        ),
        EscrowEvents::ConfirmExtended(e) => (
            order_id(e.orderId)?,
            OrderEvent::ConfirmExtended {
                confirm_deadline: e.confirmDeadline,
            },
        ),
        EscrowEvents::DeadlineExtended(e) => (
            order_id(e.orderId)?,
            OrderEvent::DeadlineExtended {
                deadline: e.deadline,
            },
        ),
        EscrowEvents::EndProposed(e) => (
            order_id(e.orderId)?,
            OrderEvent::EndProposed {
                proposer: e.proposer,
                to_buyer: e.toBuyer,
                expiry: e.expiry,
            },
        ),
        EscrowEvents::Disputed(e) => (
            order_id(e.orderId)?,
            OrderEvent::Disputed {
                by: e.by,
                fee: e.disputeFee,
                from_bond: e.fromBond,
            },
        ),
        EscrowEvents::Resolved(e) => (
            order_id(e.orderId)?,
            OrderEvent::Resolved {
                to_buyer_bps: e.toBuyerBps,
                bond_to_buyer_bps: e.bondToBuyerBps,
            },
        ),
        EscrowEvents::Settled(e) => (
            order_id(e.orderId)?,
            OrderEvent::Settled {
                state: OrderState::from_u8(e.state)?,
                to_buyer: e.toBuyer,
                to_seller: e.toSeller,
                to_fee_recipient: e.toFeeRecipient,
            },
        ),
        EscrowEvents::Credited(_)
        | EscrowEvents::EIP712DomainChanged(_)
        | EscrowEvents::Withdrawn(_)
        | EscrowEvents::OfferCancelled(_)
        | EscrowEvents::BondDeposited(_)
        | EscrowEvents::BondWithdrawn(_)
        | EscrowEvents::BondLocked(_)
        | EscrowEvents::BondReleased(_)
        | EscrowEvents::BondTaken(_) => return Ok(None),
    }))
}

/// 折叠输入：一条已入库的订单事件。
#[derive(Debug, Clone)]
pub struct OrderLog {
    pub block_number: u64,
    pub log_index: u64,
    pub level: Level,
    pub topics: Vec<B256>,
    pub data: Bytes,
    /// 仅下单事件带：下单区块的 `getOrder` 快照。
    pub snapshot: Option<OrderSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    pub proposer: Address,
    pub to_buyer: U256,
    pub expiry: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dispute {
    pub by: Address,
    pub fee: U256,
    pub from_bond: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settlement {
    pub to_buyer: U256,
    pub to_seller: U256,
    pub to_fee_recipient: U256,
}

/// 折叠结果，对应 escrow_orders 的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderView {
    pub order_id: u64,
    pub buyer: Address,
    pub seller: Address,
    pub token: Address,
    pub amount: U256,
    pub offer_hash: B256,
    pub listing_hash: B256,
    pub instant: bool,
    pub auction_id: Option<U256>,
    pub snapshot: Option<OrderSnapshot>,
    pub state: OrderState,
    pub ship_deadline: Option<u64>,
    pub confirm_deadline: Option<u64>,
    pub proposal: Option<Proposal>,
    pub dispute: Option<Dispute>,
    /// 仲裁裁决：(本金给买家的万分比, 押金给买家的万分比)。
    pub ruling: Option<(U256, U256)>,
    pub settlement: Option<Settlement>,
    /// 全部事件中最低的确认级别（最新的那条事件决定订单当前状态的可信程度）。
    pub level: Level,
    pub created_block: u64,
    pub updated_block: u64,
}

/// 按区块顺序折叠一个订单的事件。没有下单事件（被回滚，或还没补拉到）时返回 `None`。
pub fn fold(order_id: u64, logs: &[OrderLog]) -> Result<Option<OrderView>, ChainError> {
    let mut sorted: Vec<&OrderLog> = logs.iter().collect();
    sorted.sort_by_key(|l| (l.block_number, l.log_index));
    let mut view: Option<OrderView> = None;
    for log in sorted {
        let Some((id, ev)) = decode(&log.topics, &log.data)? else {
            continue;
        };
        if id != order_id {
            return Err(ChainError::Inconsistent(format!(
                "event for order {id} folded into order {order_id}"
            )));
        }
        if let OrderEvent::Created {
            buyer,
            seller,
            token,
            amount,
            offer_hash,
            listing_hash,
            instant,
        } = ev
        {
            view = Some(OrderView {
                order_id,
                buyer,
                seller,
                token,
                amount,
                offer_hash,
                listing_hash,
                instant,
                auction_id: None,
                ship_deadline: log
                    .snapshot
                    .as_ref()
                    .map(|s| s.ship_deadline)
                    .filter(|d| *d != 0),
                snapshot: log.snapshot.clone(),
                state: OrderState::Funded,
                confirm_deadline: None,
                proposal: None,
                dispute: None,
                ruling: None,
                settlement: None,
                level: log.level,
                created_block: log.block_number,
                updated_block: log.block_number,
            });
            continue;
        }
        // 下单事件之前的事件不可能出现在主链上；出现说明数据不完整，等补拉。
        let Some(v) = view.as_mut() else { continue };
        v.level = v.level.min(log.level);
        v.updated_block = log.block_number;
        match ev {
            OrderEvent::Created { .. } => {}
            OrderEvent::AuctionSettled { auction_id } => v.auction_id = Some(auction_id),
            OrderEvent::Delivered { confirm_deadline }
            | OrderEvent::ConfirmExtended { confirm_deadline } => {
                v.confirm_deadline = Some(confirm_deadline);
            }
            // 仲裁 Safe 延期：已发货延确认期，未发货延发货期（与合约 `extendDeadlines` 一致）。
            OrderEvent::DeadlineExtended { deadline } => {
                if v.confirm_deadline.is_some() {
                    v.confirm_deadline = Some(deadline);
                } else {
                    v.ship_deadline = Some(deadline);
                }
            }
            OrderEvent::EndProposed {
                proposer,
                to_buyer,
                expiry,
            } => {
                v.proposal = Some(Proposal {
                    proposer,
                    to_buyer,
                    expiry,
                });
            }
            OrderEvent::Disputed { by, fee, from_bond } => {
                v.state = OrderState::Disputed;
                v.dispute = Some(Dispute { by, fee, from_bond });
            }
            OrderEvent::Resolved {
                to_buyer_bps,
                bond_to_buyer_bps,
            } => v.ruling = Some((to_buyer_bps, bond_to_buyer_bps)),
            OrderEvent::Settled {
                state,
                to_buyer,
                to_seller,
                to_fee_recipient,
            } => {
                v.state = state;
                v.settlement = Some(Settlement {
                    to_buyer,
                    to_seller,
                    to_fee_recipient,
                });
            }
        }
    }
    Ok(view)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use alloy::primitives::address;
    use alloy::sol_types::SolEvent;

    use super::*;
    use crate::bindings::Escrow;
    use crate::event::tests::raw;
    use crate::snapshot::tests::sample;

    const BUYER: Address = address!("00000000000000000000000000000000000000b1");
    const SELLER: Address = address!("00000000000000000000000000000000000000c1");

    fn log<E: SolEvent>(ev: E, block: u64, idx: u64, level: Level) -> OrderLog {
        let r = raw(&ev, block, idx);
        OrderLog {
            block_number: block,
            log_index: idx,
            level,
            topics: r.topics,
            data: r.data,
            snapshot: None,
        }
    }

    fn created(id: u64, block: u64, level: Level) -> OrderLog {
        let mut l = log(
            Escrow::OrderCreated {
                orderId: U256::from(id),
                offerHash: B256::repeat_byte(1),
                buyer: BUYER,
                seller: SELLER,
                token: Address::ZERO,
                amount: U256::from(50_000_000u64),
                listingHash: B256::repeat_byte(2),
                instant: false,
            },
            block,
            0,
            level,
        );
        l.snapshot = Some(sample());
        l
    }

    fn delivered(id: u64, block: u64, deadline: u64) -> OrderLog {
        log(
            Escrow::Delivered {
                orderId: U256::from(id),
                confirmDeadline: deadline,
            },
            block,
            0,
            Level::Latest,
        )
    }

    fn settled(id: u64, block: u64, state: OrderState) -> OrderLog {
        log(
            Escrow::Settled {
                orderId: U256::from(id),
                state: state as u8,
                toBuyer: U256::ZERO,
                toSeller: U256::from(49_000_000u64),
                toFeeRecipient: U256::from(1_000_000u64),
            },
            block,
            1,
            Level::Latest,
        )
    }

    #[test]
    fn happy_path_folds_to_released() {
        let logs = [
            settled(1, 30, OrderState::Released),
            created(1, 10, Level::Finalized),
            delivered(1, 20, 5_000),
        ];
        let v = fold(1, &logs).unwrap().unwrap();
        assert_eq!(v.state, OrderState::Released);
        assert_eq!(v.buyer, BUYER);
        assert_eq!(v.ship_deadline, Some(1_000));
        assert_eq!(v.confirm_deadline, Some(5_000));
        assert_eq!(v.settlement.unwrap().to_seller, U256::from(49_000_000u64));
        assert_eq!(v.level, Level::Latest, "least-confirmed event wins");
        assert_eq!((v.created_block, v.updated_block), (10, 30));
    }

    #[test]
    fn without_creation_there_is_no_order() {
        assert_eq!(fold(1, &[delivered(1, 20, 5)]).unwrap(), None);
        assert_eq!(fold(1, &[]).unwrap(), None);
    }

    #[test]
    fn deadline_extension_targets_the_active_deadline() {
        let ext = |block| {
            log(
                Escrow::DeadlineExtended {
                    orderId: U256::from(1),
                    deadline: 9_999,
                },
                block,
                0,
                Level::Latest,
            )
        };
        let before = fold(1, &[created(1, 1, Level::Latest), ext(2)])
            .unwrap()
            .unwrap();
        assert_eq!(
            (before.ship_deadline, before.confirm_deadline),
            (Some(9_999), None)
        );
        let after = fold(
            1,
            &[created(1, 1, Level::Latest), delivered(1, 2, 5), ext(3)],
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            (after.ship_deadline, after.confirm_deadline),
            (Some(1_000), Some(9_999))
        );
    }

    #[test]
    fn dispute_then_resolution() {
        let disputed = log(
            Escrow::Disputed {
                orderId: U256::from(1),
                by: BUYER,
                disputeFee: U256::from(500_000u64),
                fromBond: false,
            },
            3,
            0,
            Level::Latest,
        );
        let v = fold(1, &[created(1, 1, Level::Safe), disputed.clone()])
            .unwrap()
            .unwrap();
        assert_eq!(v.state, OrderState::Disputed);
        assert_eq!(v.dispute.as_ref().unwrap().by, BUYER);
        let v = fold(
            1,
            &[
                created(1, 1, Level::Safe),
                disputed,
                settled(1, 4, OrderState::Settled),
            ],
        )
        .unwrap()
        .unwrap();
        assert_eq!(v.state, OrderState::Settled);
    }

    #[test]
    fn foreign_order_event_is_rejected() {
        assert!(fold(1, &[created(2, 1, Level::Latest)]).is_err());
    }

    #[test]
    fn non_order_events_are_ignored() {
        let credited = log(
            Escrow::Credited {
                account: BUYER,
                token: Address::ZERO,
                amount: U256::from(1),
            },
            2,
            0,
            Level::Latest,
        );
        assert!(decode(&credited.topics, &credited.data).unwrap().is_none());
        let v = fold(1, &[created(1, 1, Level::Finalized), credited])
            .unwrap()
            .unwrap();
        assert_eq!(v.level, Level::Finalized);
    }

    #[test]
    fn levels_order_and_round_trip() {
        assert!(Level::Latest < Level::Safe && Level::Safe < Level::Finalized);
        for l in [Level::Latest, Level::Safe, Level::Finalized] {
            assert_eq!(Level::parse(l.as_str()).unwrap(), l);
        }
    }
}
