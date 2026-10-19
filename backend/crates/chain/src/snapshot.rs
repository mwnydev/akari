//! `Escrow.getOrder` 的参数快照（plan 第 8 节 escrow_orders「参数快照」）。
//!
//! 下单事件只带金额与双方地址；手续费、gas、押金、各种窗口要在下单所在区块调用一次 `getOrder` 取得。
//! `Order` 有 27 个字段，超过 `sol!` 元组上限，所以用 dyn-abi 按 `abi/Escrow.getOrder.json` 编解码。

use std::collections::HashMap;
use std::sync::LazyLock;

use alloy::dyn_abi::{DynSolValue, FunctionExt, JsonAbiExt};
use alloy::json_abi::Function;
use alloy::primitives::{Address, Bytes, U256};
use serde::{Deserialize, Serialize};

use crate::ChainError;

static GET_ORDER: LazyLock<Result<Function, String>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../abi/Escrow.getOrder.json")).map_err(|e| e.to_string())
});

fn get_order() -> Result<&'static Function, ChainError> {
    GET_ORDER
        .as_ref()
        .map_err(|e| ChainError::Decode(format!("getOrder abi: {e}")))
}

/// 下单时固定下来的参数。金额以十进制字符串存入 JSON。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderSnapshot {
    pub buyer: Address,
    pub seller: Address,
    pub token: Address,
    #[serde(with = "dec")]
    pub amount: u128,
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
    pub ship_deadline: u64,
    pub confirm_deadline: u64,
    pub confirm_window: u32,
    pub resolve_timeout: u32,
    pub proposal_window: u32,
    pub late_slash_bps: u16,
}

/// `getOrder(orderId)` 的 calldata。
pub fn encode_call(order_id: u64) -> Result<Bytes, ChainError> {
    get_order()?
        .abi_encode_input(&[DynSolValue::Uint(U256::from(order_id), 256)])
        .map(Bytes::from)
        .map_err(|e| ChainError::Decode(e.to_string()))
}

/// 解码 `getOrder` 返回值。订单不存在时合约返回全零结构（state = None），这里返回 `None`。
pub fn decode_return(data: &[u8]) -> Result<Option<OrderSnapshot>, ChainError> {
    let f = get_order()?;
    let out = f
        .abi_decode_output(data)
        .map_err(|e| ChainError::Decode(format!("getOrder: {e}")))?;
    let (Some(DynSolValue::Tuple(values)), Some(param)) = (out.first(), f.outputs.first()) else {
        return Err(ChainError::Decode("getOrder: expected one tuple".into()));
    };
    let fields: HashMap<&str, &DynSolValue> = param
        .components
        .iter()
        .map(|p| p.name.as_str())
        .zip(values)
        .collect();
    let get = |name: &str| {
        fields
            .get(name)
            .copied()
            .ok_or_else(|| ChainError::Decode(format!("getOrder: no field {name}")))
    };
    let addr = |name: &str| match get(name)? {
        DynSolValue::Address(a) => Ok(*a),
        _ => Err(ChainError::Decode(format!("getOrder: {name} not address"))),
    };
    let uint = |name: &str| match get(name)? {
        DynSolValue::Uint(u, _) => {
            u128::try_from(*u).map_err(|_| ChainError::Decode(format!("getOrder: {name} overflow")))
        }
        _ => Err(ChainError::Decode(format!("getOrder: {name} not uint"))),
    };
    let narrow = |name: &str| {
        uint(name).and_then(|v| {
            u64::try_from(v).map_err(|_| ChainError::Decode(format!("getOrder: {name} overflow")))
        })
    };
    if uint("state")? == 0 {
        return Ok(None);
    }
    let small = |name: &str| {
        narrow(name).and_then(|v| {
            u32::try_from(v).map_err(|_| ChainError::Decode(format!("getOrder: {name} overflow")))
        })
    };
    Ok(Some(OrderSnapshot {
        buyer: addr("buyer")?,
        seller: addr("seller")?,
        token: addr("token")?,
        amount: uint("amount")?,
        buyer_fee: uint("buyerFee")?,
        seller_fee: uint("sellerFee")?,
        buyer_gas: uint("buyerGas")?,
        seller_gas: uint("sellerGas")?,
        dispute_fee: uint("disputeFee")?,
        bond: uint("bond")?,
        ship_deadline: narrow("shipDeadline")?,
        confirm_deadline: narrow("confirmDeadline")?,
        confirm_window: small("confirmWindow")?,
        resolve_timeout: small("resolveTimeout")?,
        proposal_window: small("proposalWindow")?,
        late_slash_bps: u16::try_from(small("lateSlashBps")?)
            .map_err(|_| ChainError::Decode("getOrder: lateSlashBps overflow".into()))?,
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
            amount: 50_000_000,
            buyer_fee: 250_000,
            seller_fee: 250_000,
            buyer_gas: 50_000,
            seller_gas: 50_000,
            dispute_fee: 500_000,
            bond: 0,
            ship_deadline: 1_000,
            confirm_deadline: 0,
            confirm_window: 86_400,
            resolve_timeout: 2_592_000,
            proposal_window: 259_200,
            late_slash_bps: 0,
        }
    }

    /// 按 ABI 字段顺序构造返回值，验证按字段名（而不是位置）取值。
    fn encode(s: &OrderSnapshot, state: u8) -> Vec<u8> {
        let f = get_order().unwrap();
        let u = |v: u128| DynSolValue::Uint(U256::from(v), 256);
        let values = f.outputs[0]
            .components
            .iter()
            .map(|p| match p.name.as_str() {
                "buyer" => DynSolValue::Address(s.buyer),
                "seller" => DynSolValue::Address(s.seller),
                "token" => DynSolValue::Address(s.token),
                "proposer" => DynSolValue::Address(Address::ZERO),
                "state" => u(state.into()),
                "amount" => u(s.amount),
                "buyerFee" => u(s.buyer_fee),
                "sellerFee" => u(s.seller_fee),
                "buyerGas" => u(s.buyer_gas),
                "sellerGas" => u(s.seller_gas),
                "disputeFee" => u(s.dispute_fee),
                "bond" => u(s.bond),
                "shipDeadline" => u(s.ship_deadline.into()),
                "confirmDeadline" => u(s.confirm_deadline.into()),
                "confirmWindow" => u(s.confirm_window.into()),
                "resolveTimeout" => u(s.resolve_timeout.into()),
                "proposalWindow" => u(s.proposal_window.into()),
                "lateSlashBps" => u(s.late_slash_bps.into()),
                _ if p.ty == "bool" => DynSolValue::Bool(false),
                _ => u(0),
            })
            .collect();
        DynSolValue::Tuple(vec![DynSolValue::Tuple(values)]).abi_encode_params()
    }

    #[test]
    fn round_trips_through_abi_and_json() {
        let s = sample();
        let got = decode_return(&encode(&s, 1)).unwrap().unwrap();
        assert_eq!(got, s);
        let json = serde_json::to_value(&got).unwrap();
        assert_eq!(json["amount"], "50000000");
        assert_eq!(serde_json::from_value::<OrderSnapshot>(json).unwrap(), s);
    }

    #[test]
    fn missing_order_is_none() {
        assert_eq!(decode_return(&encode(&sample(), 0)).unwrap(), None);
    }

    #[test]
    fn call_has_selector() {
        let data = encode_call(3).unwrap();
        assert_eq!(data.len(), 4 + 32);
        assert_eq!(data[35], 3);
    }
}
