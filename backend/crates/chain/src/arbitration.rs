//! 仲裁裁决的执行数据（plan 第 6 节「权限与参数」、ADR 0013 第 5 条）：`resolve` / `resolveTimeout` 的 calldata，
//! 以及给仲裁 Safe（3 人 2/3）签名人导入 Safe 官方界面「Transaction Builder」的批量交易文件。
//!
//! 服务端只生成数据，不持有任何签名钥匙；签名人在 Safe 界面里核对目标合约、订单号与比例后各自签名。

use alloy::primitives::{Address, Bytes, U256};
use alloy::sol_types::SolCall;
use serde::Serialize;
use serde_json::{Value, json};

use crate::bindings::{Escrow, StreamEscrow};
use crate::order::Market;

/// Safe Transaction Builder 文件格式里的工具版本（导入时只做展示）。
const TX_BUILDER_VERSION: &str = "1.18.0";

/// 一笔给仲裁 Safe 执行的调用（`value` 恒为 0，`operation` 恒为 CALL）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SafeCall {
    pub to: Address,
    pub data: Bytes,
}

/// `resolve(orderId, toBuyerBps, bondToBuyerBps)`：两份担保合约的函数签名相同，按订单所在合约选绑定。
pub fn resolve_call(
    market: Market,
    contract: Address,
    order_id: u64,
    to_buyer_bps: u32,
    bond_to_buyer_bps: u32,
) -> SafeCall {
    let (order_id, to_buyer, bond) = (
        U256::from(order_id),
        U256::from(to_buyer_bps),
        U256::from(bond_to_buyer_bps),
    );
    let data = match market {
        Market::Escrow => Escrow::resolveCall {
            orderId: order_id,
            toBuyerBps: to_buyer,
            bondToBuyerBps: bond,
        }
        .abi_encode(),
        Market::Stream => StreamEscrow::resolveCall {
            orderId: order_id,
            toBuyerBps: to_buyer,
            bondToBuyerBps: bond,
        }
        .abi_encode(),
    };
    SafeCall {
        to: contract,
        data: data.into(),
    }
}

/// `resolveTimeout(orderId)`：争议超过 `resolveTimeout` 未裁决时任何人可调用的兜底结算。
pub fn resolve_timeout_call(market: Market, contract: Address, order_id: u64) -> SafeCall {
    let order_id = U256::from(order_id);
    let data = match market {
        Market::Escrow => Escrow::resolveTimeoutCall { orderId: order_id }.abi_encode(),
        Market::Stream => StreamEscrow::resolveTimeoutCall { orderId: order_id }.abi_encode(),
    };
    SafeCall {
        to: contract,
        data: data.into(),
    }
}

/// Safe Transaction Builder 的批量文件（界面「Transaction Builder → 拖入 JSON」）。多笔会经 MultiSend 一次执行，
/// 即 plan 的「当天裁决每天合并一笔批量执行」。`name` / `description` 里写明案件编号。
pub fn transaction_builder(
    chain_id: u64,
    safe: Option<Address>,
    name: &str,
    description: &str,
    calls: &[SafeCall],
    created_at_ms: i64,
) -> Value {
    json!({
        "version": "1.0",
        "chainId": chain_id.to_string(),
        "createdAt": created_at_ms,
        "meta": {
            "name": name,
            "description": description,
            "txBuilderVersion": TX_BUILDER_VERSION,
            "createdFromSafeAddress": safe.map(|a| a.to_checksum(None)).unwrap_or_default(),
            "createdFromOwnerAddress": "",
        },
        "transactions": calls.iter().map(|c| json!({
            "to": c.to.to_checksum(None),
            "value": "0",
            "data": c.data.to_string(),
            "contractMethod": null,
            "contractInputsValues": null,
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    const CONTRACT: Address = Address::repeat_byte(0x44);

    #[test]
    fn resolve_calldata_decodes_back() {
        for market in [Market::Escrow, Market::Stream] {
            let c = resolve_call(market, CONTRACT, 42, 7500, 10_000);
            assert_eq!(c.to, CONTRACT);
            let back = Escrow::resolveCall::abi_decode(&c.data).unwrap();
            assert_eq!(back.orderId, U256::from(42));
            assert_eq!(back.toBuyerBps, U256::from(7500));
            assert_eq!(back.bondToBuyerBps, U256::from(10_000));
            // keccak256("resolve(uint256,uint256,uint256)") 的前 4 字节
            assert_eq!(&c.data[..4], Escrow::resolveCall::SELECTOR.as_slice());
            assert_eq!(c.data.len(), 4 + 32 * 3);
        }
        let t = resolve_timeout_call(Market::Stream, CONTRACT, 9);
        assert_eq!(
            StreamEscrow::resolveTimeoutCall::abi_decode(&t.data)
                .unwrap()
                .orderId,
            U256::from(9)
        );
    }

    #[test]
    fn transaction_builder_file() {
        let safe = Address::repeat_byte(0x5a);
        let calls = [resolve_call(Market::Escrow, CONTRACT, 1, 0, 0)];
        let v = transaction_builder(42_161, Some(safe), "n", "d", &calls, 1);
        assert_eq!(v["chainId"], "42161");
        assert_eq!(v["meta"]["createdFromSafeAddress"], safe.to_checksum(None));
        assert_eq!(v["transactions"][0]["to"], CONTRACT.to_checksum(None));
        assert_eq!(v["transactions"][0]["value"], "0");
        let data: Bytes = v["transactions"][0]["data"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(data, calls[0].data);
    }
}
