//! 合约日志：节点返回的原始日志与解码后的可读参数。
//!
//! 数据库里同时保存原始 topics / data（订单状态由它们经 `sol!` 类型解码后折叠，见 [`crate::order`]）
//! 和按 ABI 参数名展开的 JSON（给通知、后台与排查用）。

use std::collections::HashMap;
use std::sync::LazyLock;

use alloy::dyn_abi::{DynSolValue, EventExt};
use alloy::json_abi::{Event, Param};
use alloy::primitives::{Address, B256, Bytes};
use serde_json::{Map, Value, json};

use crate::ChainError;
use crate::bindings::{Escrow, EscrowConfig, StreamEscrow};
use crate::order::{Market, OrderEvent};

/// 一个区块的号与哈希。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockRef {
    pub number: u64,
    pub hash: B256,
}

/// 唯一标识一条日志。区块回滚后同一笔交易可能进入另一个区块，所以比较时还要看 `block_hash`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LogKey {
    pub tx_hash: B256,
    pub log_index: u64,
}

/// 节点返回的一条日志（已检查必需字段齐全）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawLog {
    pub contract: Address,
    pub block: BlockRef,
    pub key: LogKey,
    pub topics: Vec<B256>,
    pub data: Bytes,
    pub block_timestamp: Option<u64>,
    /// WebSocket 订阅在区块被回滚时推送 `removed: true`。
    pub removed: bool,
}

impl TryFrom<alloy::rpc::types::Log> for RawLog {
    type Error = ChainError;

    fn try_from(log: alloy::rpc::types::Log) -> Result<Self, ChainError> {
        let missing = |f: &str| ChainError::Inconsistent(format!("log without {f}"));
        Ok(Self {
            block: BlockRef {
                number: log.block_number.ok_or_else(|| missing("block_number"))?,
                hash: log.block_hash.ok_or_else(|| missing("block_hash"))?,
            },
            key: LogKey {
                tx_hash: log
                    .transaction_hash
                    .ok_or_else(|| missing("transaction_hash"))?,
                log_index: log.log_index.ok_or_else(|| missing("log_index"))?,
            },
            block_timestamp: log.block_timestamp,
            removed: log.removed,
            contract: log.inner.address,
            topics: log.inner.data.topics().to_vec(),
            data: log.inner.data.data.clone(),
        })
    }
}

/// 解码结果：事件名、订单类事件（所属订单号与精简形式）和可读参数。
#[derive(Debug, Clone, PartialEq)]
pub struct Decoded {
    pub name: String,
    pub order: Option<(u64, OrderEvent)>,
    pub args: Value,
}

impl Decoded {
    pub fn order_id(&self) -> Option<u64> {
        self.order.as_ref().map(|(id, _)| *id)
    }
}

type EventMap = LazyLock<HashMap<B256, Event>>;

fn events(abi: &alloy::json_abi::JsonAbi) -> HashMap<B256, Event> {
    abi.events()
        .filter(|e| !e.anonymous)
        .map(|e| (e.selector(), e.clone()))
        .collect()
}

/// 每份 ABI 的事件按 topic0 查找。两份担保合约有同名事件，按日志来自哪份合约选 ABI。
static ESCROW_EVENTS: EventMap = LazyLock::new(|| events(&Escrow::abi::contract()));
static STREAM_EVENTS: EventMap = LazyLock::new(|| events(&StreamEscrow::abi::contract()));
static CONFIG_EVENTS: EventMap = LazyLock::new(|| events(&EscrowConfig::abi::contract()));

/// 按 ABI 解码一条日志：`market` 为日志所在的担保合约，`None` = EscrowConfig。
/// 未知事件（例如合约升级后新增的）返回错误，由调用方记录后跳过。
pub fn decode(log: &RawLog, market: Option<Market>) -> Result<Decoded, ChainError> {
    let topic0 = log
        .topics
        .first()
        .ok_or_else(|| ChainError::Decode("log without topics".into()))?;
    let map = match market {
        Some(Market::Escrow) => &ESCROW_EVENTS,
        Some(Market::Stream) => &STREAM_EVENTS,
        None => &CONFIG_EVENTS,
    };
    let event = map
        .get(topic0)
        .ok_or_else(|| ChainError::Decode(format!("unknown event {topic0}")))?;
    let decoded = event
        .decode_log_parts(log.topics.iter().copied(), &log.data)
        .map_err(|e| ChainError::Decode(format!("{}: {e}", event.name)))?;
    let (mut indexed, mut body) = (decoded.indexed.into_iter(), decoded.body.into_iter());
    let mut args = Map::new();
    for input in &event.inputs {
        let value = if input.indexed {
            indexed.next()
        } else {
            body.next()
        };
        let value = value.ok_or_else(|| ChainError::Decode(format!("{}: arity", event.name)))?;
        args.insert(input.name.clone(), to_json(&value, &input.components));
    }
    let order = match market {
        Some(m) => crate::order::decode(m, &log.topics, &log.data)?,
        None => None,
    };
    Ok(Decoded {
        name: event.name.clone(),
        order,
        args: Value::Object(args),
    })
}

/// ABI 值转 JSON：整数一律十进制字符串（金额是 uint256，JSON 数字装不下），地址带校验和，结构体按字段名展开。
fn to_json(v: &DynSolValue, components: &[Param]) -> Value {
    match v {
        DynSolValue::Bool(b) => json!(b),
        DynSolValue::Int(i, _) => json!(i.to_string()),
        DynSolValue::Uint(u, _) => json!(u.to_string()),
        DynSolValue::FixedBytes(w, size) => {
            json!(format!("0x{}", alloy::hex::encode(&w[..*size])))
        }
        DynSolValue::Address(a) => json!(a.to_checksum(None)),
        DynSolValue::Function(f) => json!(f.to_string()),
        DynSolValue::Bytes(b) => json!(format!("0x{}", alloy::hex::encode(b))),
        DynSolValue::String(s) => json!(s),
        DynSolValue::Array(items) | DynSolValue::FixedArray(items) => {
            Value::Array(items.iter().map(|x| to_json(x, components)).collect())
        }
        DynSolValue::Tuple(items) if items.len() == components.len() => Value::Object(
            items
                .iter()
                .zip(components)
                .map(|(x, p)| (p.name.clone(), to_json(x, &p.components)))
                .collect(),
        ),
        DynSolValue::Tuple(items) => Value::Array(items.iter().map(|x| to_json(x, &[])).collect()),
        // 仅在启用 eip712 特性时存在的变体（CustomStruct）；合约事件里不会出现。
        #[allow(unreachable_patterns)]
        _ => Value::Null,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::unwrap_used)]

    use alloy::primitives::{U256, address, b256};
    use alloy::sol_types::SolEvent;

    use super::*;

    /// 测试用：把 `sol!` 生成的事件编码成节点会返回的日志。
    pub(crate) fn raw<E: SolEvent>(ev: &E, block: u64, log_index: u64) -> RawLog {
        let data = ev.encode_log_data();
        RawLog {
            contract: address!("00000000000000000000000000000000000000e5"),
            block: BlockRef {
                number: block,
                hash: B256::with_last_byte(u8::try_from(block % 256).unwrap()),
            },
            key: LogKey {
                tx_hash: B256::from(U256::from(block * 1000 + log_index)),
                log_index,
            },
            topics: data.topics().to_vec(),
            data: data.data,
            block_timestamp: None,
            removed: false,
        }
    }

    #[test]
    fn decodes_order_created_with_names_and_decimal_amounts() {
        let ev = Escrow::OrderCreated {
            orderId: U256::from(7),
            offerHash: b256!("1111111111111111111111111111111111111111111111111111111111111111"),
            buyer: address!("00000000000000000000000000000000000000b1"),
            seller: address!("00000000000000000000000000000000000000c1"),
            token: address!("00000000000000000000000000000000000000d1"),
            amount: U256::from(10u128.pow(30)),
            listingHash: B256::ZERO,
            instant: false,
        };
        let d = decode(&raw(&ev, 5, 0), Some(Market::Escrow)).unwrap();
        assert_eq!(d.name, "OrderCreated");
        assert_eq!(d.order_id(), Some(7));
        assert_eq!(d.args["orderId"], "7");
        assert_eq!(d.args["amount"], "1000000000000000000000000000000");
        assert_eq!(
            d.args["buyer"],
            "0x00000000000000000000000000000000000000B1"
        );
        assert_eq!(d.args["instant"], false);
    }

    #[test]
    fn decodes_config_struct_event_by_field_name() {
        let ev = EscrowConfig::CampaignSet {
            campaign: EscrowConfig::Campaign {
                start: 1,
                end: 2,
                rateBps: 50,
                scope: 3,
                gasSubsidyCap: 100,
            },
        };
        let d = decode(&raw(&ev, 1, 0), None).unwrap();
        assert_eq!(d.order_id(), None);
        assert_eq!(d.args["campaign"]["rateBps"], "50");
        assert_eq!(d.args["campaign"]["gasSubsidyCap"], "100");
    }

    #[test]
    fn non_order_escrow_event_has_no_order() {
        let ev = Escrow::Credited {
            account: Address::ZERO,
            token: Address::ZERO,
            amount: U256::from(1),
        };
        assert_eq!(
            decode(&raw(&ev, 1, 0), Some(Market::Escrow))
                .unwrap()
                .order_id(),
            None
        );
    }

    #[test]
    fn stream_events_decode_with_the_stream_abi() {
        let ev = StreamEscrow::Disputed {
            orderId: U256::from(3),
            by: Address::ZERO,
            disputeFee: U256::from(1),
            source: 2,
        };
        let log = raw(&ev, 1, 0);
        let d = decode(&log, Some(Market::Stream)).unwrap();
        assert_eq!((d.name.as_str(), d.order_id()), ("Disputed", Some(3)));
        assert_eq!(d.args["source"], "2");
        assert!(
            decode(&log, Some(Market::Escrow)).is_err(),
            "Escrow.Disputed has a different signature"
        );
    }

    #[test]
    fn unknown_event_is_an_error() {
        let mut log = raw(
            &Escrow::OfferCancelled {
                seller: Address::ZERO,
                nonce: U256::ZERO,
            },
            1,
            0,
        );
        log.topics[0] = B256::ZERO;
        assert!(decode(&log, Some(Market::Escrow)).is_err());
    }
}
