//! 担保合约 `getOrder` 的参数快照（plan 第 8 节 escrow_orders「参数快照」）。
//!
//! 下单事件只带金额与双方地址；手续费、gas、押金、各种窗口要在下单所在区块调用一次 `getOrder` 取得。
//! 两份合约的 `Order` 有 27 / 29 个字段，超过 `sol!` 元组上限，所以用 dyn-abi 按 `abi/*.getOrder.json` 编解码。

use std::collections::HashMap;
use std::sync::LazyLock;

use alloy::dyn_abi::{DynSolValue, FunctionExt, JsonAbiExt};
use alloy::json_abi::Function;
use alloy::primitives::{Address, Bytes, U256};
use serde::{Deserialize, Serialize};

use crate::ChainError;
use crate::order::Market;

type Abi = LazyLock<Result<Function, String>>;

static ESCROW_GET_ORDER: Abi = LazyLock::new(|| {
    serde_json::from_str(include_str!("../abi/Escrow.getOrder.json")).map_err(|e| e.to_string())
});
static STREAM_GET_ORDER: Abi = LazyLock::new(|| {
    serde_json::from_str(include_str!("../abi/StreamEscrow.getOrder.json"))
        .map_err(|e| e.to_string())
});

fn get_order(market: Market) -> Result<&'static Function, ChainError> {
    let abi = match market {
        Market::Escrow => &ESCROW_GET_ORDER,
        Market::Stream => &STREAM_GET_ORDER,
    };
    abi.as_ref()
        .map_err(|e| ChainError::Decode(format!("getOrder abi: {e}")))
}

/// 下单时固定下来的参数。金额以十进制字符串存入 JSON。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderSnapshot {
    pub buyer: Address,
    pub seller: Address,
    pub token: Address,
    #[serde(with = "dec")]
    pub buyer_fee: u128,
    #[serde(with = "dec")]
    pub seller_fee: u128,
    #[serde(with = "dec")]
    pub buyer_gas: u128,
    #[serde(with = "dec")]
    pub seller_gas: u128,
    #[serde(with = "dec")]
    pub dispute_fee: u128,
    #[serde(with = "dec")]
    pub bond: u128,
    pub resolve_timeout: u32,
    pub proposal_window: u32,
    pub late_slash_bps: u16,
    #[serde(flatten)]
    pub terms: Terms,
}

/// 两种模式各自的条款（JSON 里与公共字段平铺，靠字段名区分）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Terms {
    Escrow {
        #[serde(with = "dec")]
        amount: u128,
        ship_deadline: u64,
        confirm_deadline: u64,
        confirm_window: u32,
    },
    Stream {
        /// 可解锁净本金 = 金额 − 卖家承担的手续费与基础 gas。
        #[serde(with = "dec")]
        net: u128,
        #[serde(with = "dec")]
        claim_gas: u128,
        start_deadline: u64,
        interval: u32,
        periods: u32,
    },
}

impl OrderSnapshot {
    /// 一次性：发货期限（即时发货为 0）；分期：开始期限。
    pub fn deadline(&self) -> u64 {
        match self.terms {
            Terms::Escrow { ship_deadline, .. } => ship_deadline,
            Terms::Stream { start_deadline, .. } => start_deadline,
        }
    }
}

/// `getOrder(orderId)` 的 calldata。
pub fn encode_call(market: Market, order_id: u64) -> Result<Bytes, ChainError> {
    get_order(market)?
        .abi_encode_input(&[DynSolValue::Uint(U256::from(order_id), 256)])
        .map(Bytes::from)
        .map_err(|e| ChainError::Decode(e.to_string()))
}

/// 按字段名取 `Order` 结构的值（不依赖字段顺序）。
struct Fields<'a>(HashMap<&'a str, &'a DynSolValue>);

impl Fields<'_> {
    fn get(&self, name: &str) -> Result<&DynSolValue, ChainError> {
        self.0
            .get(name)
            .copied()
            .ok_or_else(|| ChainError::Decode(format!("getOrder: no field {name}")))
    }

    fn addr(&self, name: &str) -> Result<Address, ChainError> {
        match self.get(name)? {
            DynSolValue::Address(a) => Ok(*a),
            _ => Err(ChainError::Decode(format!("getOrder: {name} not address"))),
        }
    }

    fn uint<T: TryFrom<U256>>(&self, name: &str) -> Result<T, ChainError> {
        match self.get(name)? {
            DynSolValue::Uint(u, _) => T::try_from(*u)
                .map_err(|_| ChainError::Decode(format!("getOrder: {name} overflow"))),
            _ => Err(ChainError::Decode(format!("getOrder: {name} not uint"))),
        }
    }
}

/// 解码 `getOrder` 返回值。订单不存在时合约返回全零结构（state = None），这里返回 `None`。
pub fn decode_return(market: Market, data: &[u8]) -> Result<Option<OrderSnapshot>, ChainError> {
    let f = get_order(market)?;
    let out = f
        .abi_decode_output(data)
        .map_err(|e| ChainError::Decode(format!("getOrder: {e}")))?;
    let (Some(DynSolValue::Tuple(values)), Some(param)) = (out.first(), f.outputs.first()) else {
        return Err(ChainError::Decode("getOrder: expected one tuple".into()));
    };
    let o = Fields(
        param
            .components
            .iter()
            .map(|p| p.name.as_str())
            .zip(values)
            .collect(),
    );
    if o.uint::<u8>("state")? == 0 {
        return Ok(None);
    }
    let terms = match market {
        Market::Escrow => Terms::Escrow {
            amount: o.uint("amount")?,
            ship_deadline: o.uint("shipDeadline")?,
            confirm_deadline: o.uint("confirmDeadline")?,
            confirm_window: o.uint("confirmWindow")?,
        },
        Market::Stream => Terms::Stream {
            net: o.uint("net")?,
            claim_gas: o.uint("claimGas")?,
            start_deadline: o.uint("startDeadline")?,
            interval: o.uint("interval")?,
            periods: o.uint("periods")?,
        },
    };
    Ok(Some(OrderSnapshot {
        buyer: o.addr("buyer")?,
        seller: o.addr("seller")?,
        token: o.addr("token")?,
        buyer_fee: o.uint("buyerFee")?,
        seller_fee: o.uint("sellerFee")?,
        buyer_gas: o.uint("buyerGas")?,
        seller_gas: o.uint("sellerGas")?,
        dispute_fee: o.uint("disputeFee")?,
        bond: o.uint("bond")?,
        resolve_timeout: o.uint("resolveTimeout")?,
        proposal_window: o.uint("proposalWindow")?,
        late_slash_bps: o.uint("lateSlashBps")?,
        terms,
    }))
}

/// u128 ⇄ 十进制字符串。
mod dec {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &u128, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(v)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u128, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::unwrap_used)]

    use alloy::primitives::address;

    use super::*;

    pub(crate) fn sample() -> OrderSnapshot {
        OrderSnapshot {
            buyer: address!("00000000000000000000000000000000000000b1"),
            seller: address!("00000000000000000000000000000000000000c1"),
            token: address!("00000000000000000000000000000000000000d1"),
            buyer_fee: 250_000,
            seller_fee: 250_000,
            buyer_gas: 50_000,
            seller_gas: 50_000,
            dispute_fee: 500_000,
            bond: 0,
            resolve_timeout: 2_592_000,
            proposal_window: 259_200,
            late_slash_bps: 0,
            terms: Terms::Escrow {
                amount: 50_000_000,
                ship_deadline: 1_000,
                confirm_deadline: 0,
                confirm_window: 86_400,
            },
        }
    }

    pub(crate) fn stream_sample() -> OrderSnapshot {
        OrderSnapshot {
            token: address!("00000000000000000000000000000000000000d2"),
            terms: Terms::Stream {
                net: 49_450_000,
                claim_gas: 20_000,
                start_deadline: 5_000,
                interval: 3_600,
                periods: 24,
            },
            ..sample()
        }
    }

    /// 按 ABI 字段顺序构造返回值，验证按字段名（而不是位置）取值。
    fn encode(market: Market, s: &OrderSnapshot, state: u8) -> Vec<u8> {
        let f = get_order(market).unwrap();
        let u = |v: u128| DynSolValue::Uint(U256::from(v), 256);
        let mut named: HashMap<&str, DynSolValue> = HashMap::from([
            ("buyer", DynSolValue::Address(s.buyer)),
            ("seller", DynSolValue::Address(s.seller)),
            ("token", DynSolValue::Address(s.token)),
            ("state", u(state.into())),
            ("buyerFee", u(s.buyer_fee)),
            ("sellerFee", u(s.seller_fee)),
            ("buyerGas", u(s.buyer_gas)),
            ("sellerGas", u(s.seller_gas)),
            ("disputeFee", u(s.dispute_fee)),
            ("bond", u(s.bond)),
            ("resolveTimeout", u(s.resolve_timeout.into())),
            ("proposalWindow", u(s.proposal_window.into())),
            ("lateSlashBps", u(s.late_slash_bps.into())),
        ]);
        match s.terms {
            Terms::Escrow {
                amount,
                ship_deadline,
                confirm_deadline,
                confirm_window,
            } => named.extend([
                ("amount", u(amount)),
                ("shipDeadline", u(ship_deadline.into())),
                ("confirmDeadline", u(confirm_deadline.into())),
                ("confirmWindow", u(confirm_window.into())),
            ]),
            Terms::Stream {
                net,
                claim_gas,
                start_deadline,
                interval,
                periods,
            } => named.extend([
                ("net", u(net)),
                ("claimGas", u(claim_gas)),
                ("startDeadline", u(start_deadline.into())),
                ("interval", u(interval.into())),
                ("periods", u(periods.into())),
            ]),
        }
        let values = f.outputs[0]
            .components
            .iter()
            .map(|p| match named.remove(p.name.as_str()) {
                Some(v) => v,
                None if p.ty == "address" => DynSolValue::Address(Address::ZERO),
                None if p.ty == "bool" => DynSolValue::Bool(false),
                None => u(0),
            })
            .collect();
        assert!(
            named.is_empty(),
            "fields not in the ABI: {:?}",
            named.keys()
        );
        DynSolValue::Tuple(vec![DynSolValue::Tuple(values)]).abi_encode_params()
    }

    #[test]
    fn round_trips_through_abi_and_json() {
        for (market, s) in [
            (Market::Escrow, sample()),
            (Market::Stream, stream_sample()),
        ] {
            let got = decode_return(market, &encode(market, &s, 1))
                .unwrap()
                .unwrap();
            assert_eq!(got, s);
            let json = serde_json::to_value(&got).unwrap();
            assert_eq!(json["buyer_fee"], "250000");
            assert_eq!(serde_json::from_value::<OrderSnapshot>(json).unwrap(), s);
        }
    }

    #[test]
    fn json_keeps_the_escrow_format() {
        // 已入库的一次性担保快照（公共字段与条款平铺）照常读回。
        let json = serde_json::to_value(sample()).unwrap();
        assert_eq!(json["amount"], "50000000");
        assert_eq!(json["ship_deadline"], 1_000);
        let stream = serde_json::to_value(stream_sample()).unwrap();
        assert_eq!(stream["net"], "49450000");
        assert!(stream.get("amount").is_none());
    }

    #[test]
    fn missing_order_is_none() {
        for (market, s) in [
            (Market::Escrow, sample()),
            (Market::Stream, stream_sample()),
        ] {
            assert_eq!(decode_return(market, &encode(market, &s, 0)).unwrap(), None);
        }
    }

    #[test]
    fn call_has_selector() {
        let data = encode_call(Market::Stream, 3).unwrap();
        assert_eq!(data.len(), 4 + 32);
        assert_eq!(data[35], 3);
        assert_eq!(data[..4], encode_call(Market::Escrow, 3).unwrap()[..4]);
    }
}
