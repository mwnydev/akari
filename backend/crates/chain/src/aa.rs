//! ERC-4337 EntryPoint v0.7（ADR 0003）：UserOperation 的 RPC 形式、打包、哈希，以及 FixedFeePaymaster 的
//! `paymasterAndData` 布局与签名哈希（`contracts/src/aa/FixedFeePaymaster.sol`，ADR 0004）。
//!
//! 哈希在这里按合约逐字段复刻，避免每次签名都调一次 `getHash`；集成测试对照链上的 `getUserOpHash` / `getHash`。

use alloy::primitives::{Address, B256, Bytes, U256, keccak256};
use alloy::sol_types::SolValue;
use serde::{Deserialize, Serialize};

use crate::ChainError;

/// EntryPoint v0.7 在各链上的固定地址。
pub const ENTRY_POINT_V07: Address =
    alloy::primitives::address!("0x0000000071727De22E5E9d8BAf0edAc6f37da032");

alloy::sol! {
    /// EntryPoint v0.7 的打包格式（`account-abstraction/interfaces/PackedUserOperation.sol`）。
    #[derive(Debug, PartialEq, Eq)]
    struct PackedUserOperation {
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

    /// 代发器用到的 EntryPoint v0.7 接口子集（ERC-4337 标准接口，合约不在本仓库）。
    #[sol(rpc)]
    interface IEntryPoint {
        function handleOps(PackedUserOperation[] calldata ops, address payable beneficiary) external;
        function getUserOpHash(PackedUserOperation calldata userOp) external view returns (bytes32);
        function getNonce(address sender, uint192 key) external view returns (uint256 nonce);
        function balanceOf(address account) external view returns (uint256);

        event BeforeExecution();
        event UserOperationEvent(
            bytes32 indexed userOpHash,
            address indexed sender,
            address indexed paymaster,
            uint256 nonce,
            bool success,
            uint256 actualGasCost,
            uint256 actualGasUsed
        );
        event UserOperationRevertReason(
            bytes32 indexed userOpHash, address indexed sender, uint256 nonce, bytes revertReason
        );

        error FailedOp(uint256 opIndex, string reason);
        error FailedOpWithRevert(uint256 opIndex, string reason, bytes inner);
    }
}

/// `eth_sendUserOperation` 等接口里的 v0.7 未打包形式（数字为十六进制字符串）。
/// 缺省的 gas 字段按 0 处理：`pm_getPaymasterStubData` 与估算请求里它们还没有填。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserOperation {
    pub sender: Address,
    pub nonce: U256,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub factory: Option<Address>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub factory_data: Option<Bytes>,
    pub call_data: Bytes,
    #[serde(default)]
    pub call_gas_limit: U256,
    #[serde(default)]
    pub verification_gas_limit: U256,
    #[serde(default)]
    pub pre_verification_gas: U256,
    #[serde(default)]
    pub max_fee_per_gas: U256,
    #[serde(default)]
    pub max_priority_fee_per_gas: U256,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paymaster: Option<Address>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paymaster_verification_gas_limit: Option<U256>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paymaster_post_op_gas_limit: Option<U256>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paymaster_data: Option<Bytes>,
    #[serde(default)]
    pub signature: Bytes,
}

fn u128_field(v: U256, name: &str) -> Result<u128, ChainError> {
    u128::try_from(v).map_err(|_| ChainError::Decode(format!("{name} exceeds uint128")))
}

/// 两个 uint128 拼成一个 32 字节字（高位在前），与合约里 `bytes32(uint256(hi) << 128 | lo)` 相同。
fn pack_u128s(hi: u128, lo: u128) -> B256 {
    let mut out = [0u8; 32];
    out[..16].copy_from_slice(&hi.to_be_bytes());
    out[16..].copy_from_slice(&lo.to_be_bytes());
    B256::from(out)
}

impl UserOperation {
    /// `factory ‖ factoryData`；不部署钱包时为空。
    pub fn init_code(&self) -> Bytes {
        match self.factory {
            Some(f) if !f.is_zero() => {
                let mut v = f.to_vec();
                v.extend_from_slice(self.factory_data.as_ref().map_or(&[][..], |b| b.as_ref()));
                v.into()
            }
            _ => Bytes::new(),
        }
    }

    /// Paymaster 的两个 gas 上限（验证、postOp）。
    pub fn paymaster_gas(&self) -> Result<(u128, u128), ChainError> {
        Ok((
            u128_field(
                self.paymaster_verification_gas_limit.unwrap_or_default(),
                "paymasterVerificationGasLimit",
            )?,
            u128_field(
                self.paymaster_post_op_gas_limit.unwrap_or_default(),
                "paymasterPostOpGasLimit",
            )?,
        ))
    }

    /// `paymaster ‖ uint128 验证 gas ‖ uint128 postOp gas ‖ paymasterData`；没有 Paymaster 时为空。
    pub fn paymaster_and_data(&self) -> Result<Bytes, ChainError> {
        let Some(pm) = self.paymaster.filter(|p| !p.is_zero()) else {
            return Ok(Bytes::new());
        };
        let (verification, post_op) = self.paymaster_gas()?;
        let mut v = pm.to_vec();
        v.extend_from_slice(pack_u128s(verification, post_op).as_slice());
        v.extend_from_slice(self.paymaster_data.as_ref().map_or(&[][..], |b| b.as_ref()));
        Ok(v.into())
    }

    pub fn account_gas_limits(&self) -> Result<B256, ChainError> {
        Ok(pack_u128s(
            u128_field(self.verification_gas_limit, "verificationGasLimit")?,
            u128_field(self.call_gas_limit, "callGasLimit")?,
        ))
    }

    pub fn gas_fees(&self) -> Result<B256, ChainError> {
        Ok(pack_u128s(
            u128_field(self.max_priority_fee_per_gas, "maxPriorityFeePerGas")?,
            u128_field(self.max_fee_per_gas, "maxFeePerGas")?,
        ))
    }

    pub fn pack(&self) -> Result<PackedUserOperation, ChainError> {
        Ok(PackedUserOperation {
            sender: self.sender,
            nonce: self.nonce,
            initCode: self.init_code(),
            callData: self.call_data.clone(),
            accountGasLimits: self.account_gas_limits()?,
            preVerificationGas: self.pre_verification_gas,
            gasFees: self.gas_fees()?,
            paymasterAndData: self.paymaster_and_data()?,
            signature: self.signature.clone(),
        })
    }

    /// `EntryPoint.getUserOpHash`（v0.7）：不含签名的字段哈希，再绑定 EntryPoint 与链 ID。
    pub fn hash(&self, entry_point: Address, chain_id: u64) -> Result<B256, ChainError> {
        let inner = keccak256(
            (
                self.sender,
                self.nonce,
                keccak256(self.init_code()),
                keccak256(&self.call_data),
                self.account_gas_limits()?,
                self.pre_verification_gas,
                self.gas_fees()?,
                keccak256(self.paymaster_and_data()?),
            )
                .abi_encode(),
        );
        Ok(keccak256(
            (inner, entry_point, U256::from(chain_id)).abi_encode(),
        ))
    }

    /// 所有 gas 上限之和乘以 `maxFeePerGas`：EntryPoint 要求 Paymaster 押金至少覆盖这么多。
    pub fn max_cost(&self) -> Result<U256, ChainError> {
        let (pm_verification, pm_post_op) = self.paymaster_gas()?;
        let gas = self.verification_gas_limit
            + self.call_gas_limit
            + self.pre_verification_gas
            + U256::from(pm_verification)
            + U256::from(pm_post_op);
        gas.checked_mul(self.max_fee_per_gas)
            .ok_or_else(|| ChainError::Decode("max cost overflow".into()))
    }
}

/// FixedFeePaymaster 的代付条款：`paymasterData = validUntil(6) ‖ validAfter(6) ‖ fee(16) ‖ 签名`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SponsorTerms {
    pub valid_until: u64,
    pub valid_after: u64,
    pub fee: u128,
}

/// 签名在 `paymasterData` 里的起始位置。
pub const PAYMASTER_SIGNATURE_OFFSET: usize = 28;
const UINT48_MAX: u64 = (1 << 48) - 1;

impl SponsorTerms {
    /// 条款 + 签名拼成 `paymasterData`。
    pub fn encode(&self, signature: &[u8]) -> Result<Bytes, ChainError> {
        if self.valid_until > UINT48_MAX || self.valid_after > UINT48_MAX {
            return Err(ChainError::Decode("validity exceeds uint48".into()));
        }
        let mut v = Vec::with_capacity(PAYMASTER_SIGNATURE_OFFSET + signature.len());
        v.extend_from_slice(&self.valid_until.to_be_bytes()[2..]);
        v.extend_from_slice(&self.valid_after.to_be_bytes()[2..]);
        v.extend_from_slice(&self.fee.to_be_bytes());
        v.extend_from_slice(signature);
        Ok(v.into())
    }

    /// 从 `paymasterData` 读出条款（不校验签名）。
    pub fn decode(data: &[u8]) -> Result<Self, ChainError> {
        let short = || ChainError::Decode("paymasterData too short".into());
        let head = data.get(..PAYMASTER_SIGNATURE_OFFSET).ok_or_else(short)?;
        let u48 = |b: &[u8]| {
            let mut buf = [0u8; 8];
            buf[2..].copy_from_slice(b);
            u64::from_be_bytes(buf)
        };
        let mut fee = [0u8; 16];
        fee.copy_from_slice(&head[12..28]);
        Ok(Self {
            valid_until: u48(&head[..6]),
            valid_after: u48(&head[6..12]),
            fee: u128::from_be_bytes(fee),
        })
    }

    /// `FixedFeePaymaster.getHash`：签名者对它再加 EIP-191 前缀签名（`toEthSignedMessageHash`）。
    /// `op` 里的 Paymaster gas 上限必须是最终值（它们在哈希里），`paymasterData` 不参与。
    pub fn hash(
        &self,
        op: &UserOperation,
        chain_id: u64,
        paymaster: Address,
    ) -> Result<B256, ChainError> {
        let (pm_verification, pm_post_op) = op.paymaster_gas()?;
        let op_hash = keccak256(
            (
                op.sender,
                op.nonce,
                keccak256(op.init_code()),
                keccak256(&op.call_data),
                op.account_gas_limits()?,
                U256::from_be_bytes(pack_u128s(pm_verification, pm_post_op).0),
                op.pre_verification_gas,
                op.gas_fees()?,
            )
                .abi_encode(),
        );
        Ok(keccak256(
            (
                op_hash,
                U256::from(chain_id),
                paymaster,
                U256::from(self.valid_until),
                U256::from(self.valid_after),
                U256::from(self.fee),
            )
                .abi_encode(),
        ))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn op() -> UserOperation {
        serde_json::from_value(serde_json::json!({
            "sender": "0x1111111111111111111111111111111111111111",
            "nonce": "0x5",
            "factory": "0x2222222222222222222222222222222222222222",
            "factoryData": "0xabcd",
            "callData": "0x1234",
            "callGasLimit": "0x10",
            "verificationGasLimit": "0x20",
            "preVerificationGas": "0x30",
            "maxFeePerGas": "0x40",
            "maxPriorityFeePerGas": "0x1",
            "paymaster": "0x3333333333333333333333333333333333333333",
            "paymasterVerificationGasLimit": "0x50",
            "paymasterPostOpGasLimit": "0x0",
            "paymasterData": "0x99",
            "signature": "0x",
            "eip7702Auth": null
        }))
        .unwrap()
    }

    #[test]
    fn packs_v07_fields() {
        let p = op().pack().unwrap();
        assert_eq!(
            p.initCode.to_vec(),
            [vec![0x22; 20], vec![0xab, 0xcd]].concat()
        );
        let mut limits = [0u8; 32];
        limits[15] = 0x20;
        limits[31] = 0x10;
        assert_eq!(p.accountGasLimits, B256::from(limits));
        let mut fees = [0u8; 32];
        fees[15] = 0x01;
        fees[31] = 0x40;
        assert_eq!(p.gasFees, B256::from(fees));
        assert_eq!(p.paymasterAndData.len(), 20 + 32 + 1);
        assert_eq!(p.paymasterAndData[35], 0x50);
        assert_eq!(p.paymasterAndData[52], 0x99);
        assert_eq!(
            op().max_cost().unwrap(),
            U256::from((0x10 + 0x20 + 0x30 + 0x50) * 0x40)
        );
    }

    #[test]
    fn missing_factory_and_paymaster_pack_empty() {
        let mut o = op();
        o.factory = None;
        o.paymaster = None;
        let p = o.pack().unwrap();
        assert!(p.initCode.is_empty());
        assert!(p.paymasterAndData.is_empty());
    }

    #[test]
    fn oversized_gas_is_rejected() {
        let mut o = op();
        o.call_gas_limit = U256::from(u128::MAX) + U256::from(1);
        assert!(o.pack().is_err());
    }

    #[test]
    fn terms_roundtrip_and_layout() {
        let t = SponsorTerms {
            valid_until: 1_800_000_120,
            valid_after: 1_800_000_000,
            fee: 50_000,
        };
        let data = t.encode(&[7u8; 65]).unwrap();
        assert_eq!(data.len(), PAYMASTER_SIGNATURE_OFFSET + 65);
        assert_eq!(SponsorTerms::decode(&data).unwrap(), t);
        assert_eq!(&data[12..28], &50_000u128.to_be_bytes());
        assert!(SponsorTerms::decode(&data[..27]).is_err());
    }

    #[test]
    fn hashes_change_with_every_signed_field() {
        let t = SponsorTerms {
            valid_until: 2,
            valid_after: 1,
            fee: 3,
        };
        let pm = Address::repeat_byte(0x33);
        let base = t.hash(&op(), 1, pm).unwrap();
        let mut o = op();
        o.paymaster_data = Some(Bytes::from_static(&[1, 2, 3]));
        assert_eq!(
            t.hash(&o, 1, pm).unwrap(),
            base,
            "paymasterData is not signed"
        );
        o.call_data = Bytes::from_static(&[0]);
        assert_ne!(t.hash(&o, 1, pm).unwrap(), base);
        assert_ne!(t.hash(&op(), 2, pm).unwrap(), base);
        assert_ne!(
            SponsorTerms { fee: 4, ..t }.hash(&op(), 1, pm).unwrap(),
            base
        );
        let user = op().hash(ENTRY_POINT_V07, 1).unwrap();
        let mut o = op();
        o.signature = Bytes::from_static(&[9]);
        assert_eq!(
            o.hash(ENTRY_POINT_V07, 1).unwrap(),
            user,
            "signature is not hashed"
        );
        o.paymaster_data = Some(Bytes::from_static(&[1]));
        assert_ne!(o.hash(ENTRY_POINT_V07, 1).unwrap(), user);
    }
}
