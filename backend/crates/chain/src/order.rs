//! 订单状态 = 该订单全部事件按 (区块号, 日志序号) 折叠的结果。
//!
//! 折叠是纯函数：区块回滚时删掉分叉上的事件、重放主链事件，再折叠一次就得到正确状态，不需要「反向操作」。

use alloy::primitives::{Address, B256, Bytes, U256};
use alloy::sol_types::SolEventInterface;

use crate::ChainError;
use crate::bindings::Escrow::EscrowEvents;
use crate::bindings::StreamEscrow::StreamEscrowEvents;
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

    /// 终态：已放款、已退款或已结算（含仲裁裁决与超时裁决），资金已分配完毕。
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Released | Self::Refunded | Self::Settled)
    }
}

/// 订单所在的担保合约（`escrow_orders.mode`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Market {
    /// 一次性担保 `Escrow`。
    Escrow,
    /// 分期担保 `StreamEscrow`。
    Stream,
}

impl Market {
    /// 与数据库列 `escrow_orders.mode` 的取值一致。
    pub fn mode(self) -> i16 {
        match self {
            Self::Escrow => 0,
            Self::Stream => 1,
        }
    }

    pub fn from_mode(mode: i16) -> Result<Self, ChainError> {
        match mode {
            0 => Ok(Self::Escrow),
            1 => Ok(Self::Stream),
            _ => Err(ChainError::Decode(format!("unknown order mode {mode}"))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Escrow => "escrow",
            Self::Stream => "stream",
        }
    }
}

/// 争议费从哪里付（一次性担保只有钱包与保证金两种）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisputeSource {
    Wallet,
    /// 分期：从本单已解锁未领取的本金里扣（计入已领取）。
    Unlocked,
    Bond,
}

impl DisputeSource {
    /// 与 `StreamEscrow.FeeSource` 一致。
    fn from_stream(v: u8) -> Result<Self, ChainError> {
        Ok(match v {
            0 => Self::Wallet,
            1 => Self::Unlocked,
            2 => Self::Bond,
            _ => return Err(ChainError::Decode(format!("unknown fee source {v}"))),
        })
    }
}

/// 分期条款：每期秒数与期数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamTerms {
    pub interval: u32,
    pub periods: u32,
}

/// 影响订单状态的事件（`sol!` 类型解码后的精简形式）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderEvent {
    Created {
        buyer: Address,
        seller: Address,
        /// 分期的下单事件不带代币，取自下单快照。
        token: Option<Address>,
        amount: U256,
        offer_hash: B256,
        listing_hash: B256,
        instant: bool,
        stream: Option<StreamTerms>,
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
    Started {
        started_at: u64,
    },
    Claimed {
        to_seller: U256,
        claim_gas: U256,
    },
    EndProposed {
        proposer: Address,
        to_buyer: U256,
        expiry: u64,
    },
    Disputed {
        by: Address,
        fee: U256,
        source: DisputeSource,
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

impl OrderEvent {
    /// 折叠要用到所在区块的时间（争议时间：分期解锁冻结在这一刻）。
    pub fn needs_block_time(&self) -> bool {
        matches!(self, Self::Disputed { .. })
    }
}

fn order_id(id: U256) -> Result<u64, ChainError> {
    u64::try_from(id).map_err(|_| ChainError::Decode(format!("order id {id} exceeds u64")))
}

/// 解码订单类事件；其他事件（押金池、配置、提现等）返回 `None`。
pub fn decode(
    market: Market,
    topics: &[B256],
    data: &[u8],
) -> Result<Option<(u64, OrderEvent)>, ChainError> {
    let Some(topic0) = topics.first() else {
        return Ok(None);
    };
    match market {
        Market::Escrow if EscrowEvents::SELECTORS.contains(&topic0.0) => {
            let ev = EscrowEvents::decode_raw_log(topics, data)
                .map_err(|e| ChainError::Decode(format!("escrow event: {e}")))?;
            decode_escrow(ev)
        }
        Market::Stream if StreamEscrowEvents::SELECTORS.contains(&topic0.0) => {
            let ev = StreamEscrowEvents::decode_raw_log(topics, data)
                .map_err(|e| ChainError::Decode(format!("stream escrow event: {e}")))?;
            decode_stream(ev)
        }
        _ => Ok(None),
    }
}

fn decode_escrow(ev: EscrowEvents) -> Result<Option<(u64, OrderEvent)>, ChainError> {
    Ok(Some(match ev {
        EscrowEvents::OrderCreated(e) => (
            order_id(e.orderId)?,
            OrderEvent::Created {
                buyer: e.buyer,
                seller: e.seller,
                token: Some(e.token),
                amount: e.amount,
                offer_hash: e.offerHash,
                listing_hash: e.listingHash,
                instant: e.instant,
                stream: None,
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
                source: if e.fromBond {
                    DisputeSource::Bond
                } else {
                    DisputeSource::Wallet
                },
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

fn decode_stream(ev: StreamEscrowEvents) -> Result<Option<(u64, OrderEvent)>, ChainError> {
    Ok(Some(match ev {
        StreamEscrowEvents::OrderCreated(e) => (
            order_id(e.orderId)?,
            OrderEvent::Created {
                buyer: e.buyer,
                seller: e.seller,
                token: None,
                amount: e.amount,
                offer_hash: e.offerHash,
                listing_hash: e.listingHash,
                instant: false,
                stream: Some(StreamTerms {
                    interval: e.interval,
                    periods: e.periods,
                }),
            },
        ),
        StreamEscrowEvents::Started(e) => (
            order_id(e.orderId)?,
            OrderEvent::Started {
                started_at: e.startedAt,
            },
        ),
        StreamEscrowEvents::Claimed(e) => (
            order_id(e.orderId)?,
            OrderEvent::Claimed {
                to_seller: e.toSeller,
                claim_gas: e.claimGas,
            },
        ),
        StreamEscrowEvents::DeadlineExtended(e) => (
            order_id(e.orderId)?,
            OrderEvent::DeadlineExtended {
                deadline: e.deadline,
            },
        ),
        StreamEscrowEvents::EndProposed(e) => (
            order_id(e.orderId)?,
            OrderEvent::EndProposed {
                proposer: e.proposer,
                to_buyer: e.toBuyer,
                expiry: e.expiry,
            },
        ),
        StreamEscrowEvents::Disputed(e) => (
            order_id(e.orderId)?,
            OrderEvent::Disputed {
                by: e.by,
                fee: e.disputeFee,
                source: DisputeSource::from_stream(e.source)?,
            },
        ),
        StreamEscrowEvents::Resolved(e) => (
            order_id(e.orderId)?,
            OrderEvent::Resolved {
                to_buyer_bps: e.toBuyerBps,
                bond_to_buyer_bps: e.bondToBuyerBps,
            },
        ),
        StreamEscrowEvents::Settled(e) => (
            order_id(e.orderId)?,
            OrderEvent::Settled {
                state: OrderState::from_u8(e.state)?,
                to_buyer: e.toBuyer,
                to_seller: e.toSeller,
                to_fee_recipient: e.toFeeRecipient,
            },
        ),
        StreamEscrowEvents::Credited(_)
        | StreamEscrowEvents::EIP712DomainChanged(_)
        | StreamEscrowEvents::Withdrawn(_)
        | StreamEscrowEvents::OfferCancelled(_) => return Ok(None),
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
    /// 区块时间（Unix 秒）；只保证 [`OrderEvent::needs_block_time`] 的事件有。
    pub block_time: Option<u64>,
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
    pub source: DisputeSource,
    /// 发起争议的区块时间（Unix 秒）。
    pub at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settlement {
    pub to_buyer: U256,
    pub to_seller: U256,
    pub to_fee_recipient: U256,
}

/// 分期订单的进度。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamProgress {
    pub terms: StreamTerms,
    /// 卖家开始服务的时间；未开始为 `None`。
    pub started_at: Option<u64>,
    /// 已领取的可解锁净本金（与合约 `Order.claimed` 一致：含领取 gas 与从已解锁部分扣的争议费）。
    pub claimed: U256,
}

/// 折叠结果，对应 escrow_orders 的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderView {
    pub market: Market,
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
    /// 一次性：发货期限；分期：开始期限。
    pub ship_deadline: Option<u64>,
    pub confirm_deadline: Option<u64>,
    pub stream: Option<StreamProgress>,
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
pub fn fold(
    market: Market,
    order_id: u64,
    logs: &[OrderLog],
) -> Result<Option<OrderView>, ChainError> {
    let mut sorted: Vec<&OrderLog> = logs.iter().collect();
    sorted.sort_by_key(|l| (l.block_number, l.log_index));
    let mut view: Option<OrderView> = None;
    for log in sorted {
        let Some((id, ev)) = decode(market, &log.topics, &log.data)? else {
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
            stream,
        } = ev
        {
            let snapshot = log.snapshot.as_ref();
            view = Some(OrderView {
                market,
                order_id,
                buyer,
                seller,
                // 分期快照缺失（节点异常）时没有代币地址：记为零地址，不阻塞其余字段。
                token: token
                    .or_else(|| snapshot.map(|s| s.token))
                    .unwrap_or_default(),
                amount,
                offer_hash,
                listing_hash,
                instant,
                auction_id: None,
                ship_deadline: snapshot.map(OrderSnapshot::deadline).filter(|d| *d != 0),
                snapshot: log.snapshot.clone(),
                state: OrderState::Funded,
                confirm_deadline: None,
                stream: stream.map(|terms| StreamProgress {
                    terms,
                    started_at: None,
                    claimed: U256::ZERO,
                }),
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
            // 仲裁 Safe 延期：已发货延确认期，未发货（或分期未开始）延发货 / 开始期限（与合约 `extendDeadlines` 一致）。
            OrderEvent::DeadlineExtended { deadline } => {
                if v.confirm_deadline.is_some() {
                    v.confirm_deadline = Some(deadline);
                } else {
                    v.ship_deadline = Some(deadline);
                }
            }
            OrderEvent::Started { started_at } => {
                if let Some(s) = v.stream.as_mut() {
                    s.started_at = Some(started_at);
                }
            }
            OrderEvent::Claimed {
                to_seller,
                claim_gas,
            } => {
                if let Some(s) = v.stream.as_mut() {
                    s.claimed = s
                        .claimed
                        .saturating_add(to_seller)
                        .saturating_add(claim_gas);
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
            OrderEvent::Disputed { by, fee, source } => {
                v.state = OrderState::Disputed;
                if source == DisputeSource::Unlocked
                    && let Some(s) = v.stream.as_mut()
                {
                    s.claimed = s.claimed.saturating_add(fee);
                }
                v.dispute = Some(Dispute {
                    by,
                    fee,
                    source,
                    at: log.block_time,
                });
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
    use crate::bindings::{Escrow, StreamEscrow};
    use crate::event::tests::raw;
    use crate::snapshot::tests::{sample, stream_sample};

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
            block_time: None,
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
        let v = fold(Market::Escrow, 1, &logs).unwrap().unwrap();
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
        assert_eq!(
            fold(Market::Escrow, 1, &[delivered(1, 20, 5)]).unwrap(),
            None
        );
        assert_eq!(fold(Market::Escrow, 1, &[]).unwrap(), None);
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
        let before = fold(Market::Escrow, 1, &[created(1, 1, Level::Latest), ext(2)])
            .unwrap()
            .unwrap();
        assert_eq!(
            (before.ship_deadline, before.confirm_deadline),
            (Some(9_999), None)
        );
        let after = fold(
            Market::Escrow,
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
        let mut disputed = log(
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
        disputed.block_time = Some(7_777);
        let v = fold(
            Market::Escrow,
            1,
            &[created(1, 1, Level::Safe), disputed.clone()],
        )
        .unwrap()
        .unwrap();
        assert_eq!(v.state, OrderState::Disputed);
        let d = v.dispute.as_ref().unwrap();
        assert_eq!(
            (d.by, d.source, d.at),
            (BUYER, DisputeSource::Wallet, Some(7_777))
        );
        let v = fold(
            Market::Escrow,
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
        assert!(fold(Market::Escrow, 1, &[created(2, 1, Level::Latest)]).is_err());
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
        assert!(
            decode(Market::Escrow, &credited.topics, &credited.data)
                .unwrap()
                .is_none()
        );
        let v = fold(
            Market::Escrow,
            1,
            &[created(1, 1, Level::Finalized), credited],
        )
        .unwrap()
        .unwrap();
        assert_eq!(v.level, Level::Finalized);
    }

    fn stream_created(block: u64) -> OrderLog {
        let mut l = log(
            StreamEscrow::OrderCreated {
                orderId: U256::from(1),
                offerHash: B256::repeat_byte(1),
                buyer: BUYER,
                seller: SELLER,
                amount: U256::from(50_000_000u64),
                listingHash: B256::repeat_byte(2),
                interval: 3_600,
                periods: 24,
            },
            block,
            0,
            Level::Latest,
        );
        l.snapshot = Some(stream_sample());
        l
    }

    fn stream_log<E: SolEvent>(ev: E, block: u64) -> OrderLog {
        log(ev, block, 0, Level::Latest)
    }

    #[test]
    fn stream_start_claim_and_dispute_from_unlocked() {
        let id = U256::from(1);
        let mut disputed = stream_log(
            StreamEscrow::Disputed {
                orderId: id,
                by: SELLER,
                disputeFee: U256::from(1_000_000u64),
                source: 1, // FeeSource.Unlocked
            },
            5,
        );
        disputed.block_time = Some(20_000);
        let logs = [
            stream_created(1),
            stream_log(
                StreamEscrow::Started {
                    orderId: id,
                    startedAt: 10_000,
                    endsAt: 10_000 + 24 * 3_600,
                },
                2,
            ),
            stream_log(
                StreamEscrow::Claimed {
                    orderId: id,
                    toSeller: U256::from(4_000_000u64),
                    claimGas: U256::from(20_000u64),
                },
                3,
            ),
            disputed,
        ];
        let v = fold(Market::Stream, 1, &logs).unwrap().unwrap();
        assert_eq!(v.market, Market::Stream);
        assert_eq!(
            v.token,
            stream_sample().token,
            "stream token comes from the snapshot"
        );
        assert_eq!(v.ship_deadline, Some(5_000), "start deadline");
        let s = v.stream.as_ref().unwrap();
        assert_eq!(
            s.terms,
            StreamTerms {
                interval: 3_600,
                periods: 24
            }
        );
        assert_eq!(s.started_at, Some(10_000));
        assert_eq!(
            s.claimed,
            U256::from(5_020_000u64),
            "claims + gas + fee taken from unlocked"
        );
        let d = v.dispute.unwrap();
        assert_eq!((d.source, d.at), (DisputeSource::Unlocked, Some(20_000)));
        assert_eq!(v.state, OrderState::Disputed);
    }

    #[test]
    fn stream_deadline_extension_moves_start_deadline() {
        let ext = stream_log(
            StreamEscrow::DeadlineExtended {
                orderId: U256::from(1),
                deadline: 9_999,
            },
            2,
        );
        let v = fold(Market::Stream, 1, &[stream_created(1), ext])
            .unwrap()
            .unwrap();
        assert_eq!((v.ship_deadline, v.confirm_deadline), (Some(9_999), None));
    }

    #[test]
    fn market_decides_the_abi() {
        // 同一条分期下单日志按一次性担保的 ABI 不是订单事件（签名不同）。
        let l = stream_created(1);
        assert!(
            decode(Market::Escrow, &l.topics, &l.data)
                .unwrap()
                .is_none()
        );
        assert!(
            decode(Market::Stream, &l.topics, &l.data)
                .unwrap()
                .is_some()
        );
        for m in [Market::Escrow, Market::Stream] {
            assert_eq!(Market::from_mode(m.mode()).unwrap(), m);
        }
    }

    #[test]
    fn levels_order_and_round_trip() {
        assert!(Level::Latest < Level::Safe && Level::Safe < Level::Finalized);
        for l in [Level::Latest, Level::Safe, Level::Finalized] {
            assert_eq!(Level::parse(l.as_str()).unwrap(), l);
        }
    }
}
