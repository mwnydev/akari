//! Kernel v3.3（ADR 0003）`callData` 解码：ERC-7579 `execute(bytes32 mode, bytes executionCalldata)`。
//!
//! 只接受普通调用的单笔（callType 0x00）与批量（0x01）、默认执行方式（execType 0x00，任一笔失败整体回滚），
//! 模式其余字节必须为 0。delegatecall（0xff）、try 模式、`executeUserOp` 等其他入口一律拒绝。

use alloy::primitives::{Address, Bytes, U256};
use alloy::sol_types::{SolCall, SolValue};

use crate::jsonrpc::Reject;

alloy::sol! {
    /// ERC-7579 批量执行的一项。
    struct Execution {
        address target;
        uint256 value;
        bytes callData;
    }

    /// Kernel v3.3 中本服务认识的入口。签名者变更在 Kernel 上就是安装 / 卸载验证器。
    interface IKernel {
        function execute(bytes32 mode, bytes calldata executionCalldata) external payable;
        function installValidations(
            bytes21[] calldata vIds,
            ValidationConfig[] calldata configs,
            bytes[] calldata validationData,
            bytes[] calldata hookData
        ) external payable;
        function uninstallValidation(bytes21 vId, bytes calldata deinitData, bytes calldata hookDeinitData)
            external payable;
    }

    struct ValidationConfig {
        uint32 nonce;
        address hook;
    }
}

const CALL_TYPE_SINGLE: u8 = 0x00;
const CALL_TYPE_BATCH: u8 = 0x01;

/// 账户发出的一笔调用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub target: Address,
    pub value: U256,
    pub data: Bytes,
}

/// 解出 `callData` 里的全部调用。
pub fn decode(call_data: &[u8]) -> Result<Vec<Call>, Reject> {
    let exec = IKernel::executeCall::abi_decode(call_data)
        .map_err(|_| Reject::not_allowed("callData is not Kernel execute(bytes32,bytes)"))?;
    let mode = exec.mode.0;
    if mode[1..].iter().any(|&b| b != 0) {
        return Err(Reject::not_allowed("unsupported execution mode"));
    }
    let data = exec.executionCalldata;
    match mode[0] {
        CALL_TYPE_SINGLE => {
            if data.len() < 52 {
                return Err(Reject::not_allowed("single execution too short"));
            }
            Ok(vec![Call {
                target: Address::from_slice(&data[..20]),
                value: U256::from_be_slice(&data[20..52]),
                data: Bytes::copy_from_slice(&data[52..]),
            }])
        }
        CALL_TYPE_BATCH => {
            let execs = <Vec<Execution>>::abi_decode(&data)
                .map_err(|_| Reject::not_allowed("malformed batch execution"))?;
            Ok(execs
                .into_iter()
                .map(|e| Call {
                    target: e.target,
                    value: e.value,
                    data: e.callData,
                })
                .collect())
        }
        _ => Err(Reject::not_allowed("unsupported call type")),
    }
}

/// 按 Kernel 规则编码（测试与集成测试用）。
pub fn encode(calls: &[Call]) -> Bytes {
    let mut mode = [0u8; 32];
    let data: Vec<u8> = if let [c] = calls {
        mode[0] = CALL_TYPE_SINGLE;
        [c.target.as_slice(), &c.value.to_be_bytes::<32>(), &c.data].concat()
    } else {
        mode[0] = CALL_TYPE_BATCH;
        calls
            .iter()
            .map(|c| Execution {
                target: c.target,
                value: c.value,
                callData: c.data.clone(),
            })
            .collect::<Vec<_>>()
            .abi_encode()
    };
    IKernel::executeCall {
        mode: mode.into(),
        executionCalldata: data.into(),
    }
    .abi_encode()
    .into()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn call(n: u8) -> Call {
        Call {
            target: Address::repeat_byte(n),
            value: U256::ZERO,
            data: Bytes::from(vec![n; 4 + n as usize]),
        }
    }

    #[test]
    fn selector_is_erc7579_execute() {
        assert_eq!(IKernel::executeCall::SELECTOR, [0xe9, 0xae, 0x5c, 0x53]);
    }

    #[test]
    fn single_and_batch_roundtrip() {
        let one = vec![call(1)];
        assert_eq!(decode(&encode(&one)).unwrap(), one);
        let many = vec![call(1), call(2), call(3)];
        assert_eq!(decode(&encode(&many)).unwrap(), many);
    }

    #[test]
    fn rejects_other_modes_and_entry_points() {
        let mut data = encode(&[call(1)]).to_vec();
        data[4] = 0xff; // delegatecall
        assert!(decode(&data).is_err());
        let mut data = encode(&[call(1)]).to_vec();
        data[5] = 0x01; // try 模式
        assert!(decode(&data).is_err());
        let mut data = encode(&[call(1)]).to_vec();
        data[10] = 0x01; // mode selector
        assert!(decode(&data).is_err());
        assert!(decode(&[0x8d, 0xd7, 0x71, 0x2f, 0, 0]).is_err());
        assert!(decode(&[]).is_err());
    }

    #[test]
    fn rejects_truncated_single_execution() {
        let exec = IKernel::executeCall {
            mode: [0u8; 32].into(),
            executionCalldata: vec![1u8; 51].into(),
        }
        .abi_encode();
        assert!(decode(&exec).is_err());
    }
}
