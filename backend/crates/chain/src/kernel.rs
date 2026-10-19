//! Kernel v3.3 智能账户的 counterfactual 地址（ADR 0003、0008）。
//!
//! 初始化数据与钱包前端（ZeroDev SDK `createKernelAccount`，WebAuthn 验证器 0.0.3）逐字节相同：
//! `initialize(0x01 ‖ 验证器, 无 hook, abi.encode((x, y), keccak256(凭证 ID)), 0x, [授权调用])`，
//! 其中授权调用 = `execute(单笔默认模式, 代币 ‖ 0 ‖ approve(paymaster, 额度))`，账户序号（salt）为 0。
//! 地址由版本里的工厂 `getAddress(initData, salt)` 算出：CREATE2，账户未部署也能算。

use alloy::primitives::{Address, B256, Bytes, FixedBytes, TxKind, U256, keccak256};
use alloy::providers::{DynProvider, Provider};
use alloy::rpc::types::{TransactionInput, TransactionRequest};
use alloy::sol_types::{SolCall, SolValue};

use crate::ChainError;
use crate::config::{ChainConfig, WalletInitVersion};
use crate::rpc::FailoverRpc;

alloy::sol! {
    interface IKernel {
        function initialize(bytes21 rootValidator, address hook, bytes validatorData, bytes hookData, bytes[] initConfig);
        function execute(bytes32 execMode, bytes executionCalldata);
    }

    interface IKernelFactory {
        function getAddress(bytes data, bytes32 salt) external view returns (address);
        function implementation() external view returns (address);
    }

    interface IApprove {
        function approve(address spender, uint256 amount) external returns (bool);
    }
}

/// Kernel v3 的验证器类型字节（`ValidationId = 类型 ‖ 验证器地址`）。
const VALIDATOR_TYPE: u8 = 0x01;
/// 每个 Passkey 只用第 0 号账户。
pub const ACCOUNT_SALT: B256 = B256::ZERO;

/// 钱包 Passkey 的 P-256 公钥与凭证 ID 哈希（WebAuthn 验证器的 `authenticatorIdHash`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PasskeyPublicKey {
    pub x: B256,
    pub y: B256,
    pub authenticator_id_hash: B256,
}

impl PasskeyPublicKey {
    /// `credential_id` 为原始字节（不是 base64url 文本）。
    pub fn new(x: [u8; 32], y: [u8; 32], credential_id: &[u8]) -> Self {
        Self {
            x: B256::from(x),
            y: B256::from(y),
            authenticator_id_hash: keccak256(credential_id),
        }
    }
}

/// Kernel `initialize(...)` 的 calldata，即工厂 `createAccount` / `getAddress` 的 `data`。
pub fn init_data(v: &WalletInitVersion, key: &PasskeyPublicKey) -> Bytes {
    let mut root = [0u8; 21];
    root[0] = VALIDATOR_TYPE;
    root[1..].copy_from_slice(v.webauthn_validator.as_slice());
    let validator_data = (
        (U256::from_be_bytes(key.x.0), U256::from_be_bytes(key.y.0)),
        key.authenticator_id_hash,
    )
        .abi_encode_params();
    let approve = IApprove::approveCall {
        spender: v.paymaster,
        amount: v.approve_amount,
    }
    .abi_encode();
    // ERC-7579 单笔调用：目标地址 ‖ value（32 字节）‖ calldata，紧密拼接。
    let execution = [
        v.approve_token.as_slice(),
        &U256::ZERO.to_be_bytes::<32>(),
        &approve,
    ]
    .concat();
    let init_config = IKernel::executeCall {
        execMode: B256::ZERO,
        executionCalldata: execution.into(),
    }
    .abi_encode();
    IKernel::initializeCall {
        rootValidator: FixedBytes(root),
        hook: Address::ZERO,
        validatorData: validator_data.into(),
        hookData: Bytes::new(),
        initConfig: vec![init_config.into()],
    }
    .abi_encode()
    .into()
}

async fn eth_call<C: SolCall>(
    provider: &DynProvider,
    to: Address,
    call: &C,
) -> Result<C::Return, ChainError> {
    let tx = TransactionRequest {
        to: Some(TxKind::Call(to)),
        input: TransactionInput::new(call.abi_encode().into()),
        ..Default::default()
    };
    let out = provider.call(tx).await?;
    C::abi_decode_returns(&out).map_err(|e| ChainError::Decode(format!("{}: {e}", C::SIGNATURE)))
}

/// 在一个节点上推算地址。工厂的 Kernel 实现与版本不符时报配置错误（地址会对不上前端）。
pub async fn counterfactual_address(
    provider: &DynProvider,
    v: &WalletInitVersion,
    key: &PasskeyPublicKey,
) -> Result<Address, ChainError> {
    let implementation = eth_call(
        provider,
        v.kernel_factory,
        &IKernelFactory::implementationCall {},
    )
    .await?;
    if implementation != v.kernel_implementation {
        return Err(ChainError::Config(format!(
            "wallet init version {}: factory implementation is {implementation}, configured {}",
            v.version, v.kernel_implementation
        )));
    }
    eth_call(
        provider,
        v.kernel_factory,
        &IKernelFactory::getAddressCall {
            data: init_data(v, key),
            salt: ACCOUNT_SALT,
        },
    )
    .await
}

/// 按链配置的主备节点推算钱包地址（主备切换见 [`FailoverRpc`]）。
pub struct AddressResolver {
    rpc: FailoverRpc,
}

impl AddressResolver {
    pub fn new(cfg: &ChainConfig) -> Result<Self, ChainError> {
        Ok(Self {
            rpc: FailoverRpc::from_config(cfg)?,
        })
    }

    pub async fn address(
        &self,
        v: &WalletInitVersion,
        key: &PasskeyPublicKey,
    ) -> Result<Address, ChainError> {
        self.rpc
            .call("kernel getAddress", |p| async move {
                match counterfactual_address(&p, v, key).await {
                    // 配置错误换节点也一样，不触发切换。
                    Err(e @ ChainError::Config(_)) => Ok(Err(e)),
                    r => r.map(Ok),
                }
            })
            .await?
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use alloy::primitives::hex;

    use super::*;
    use crate::anvil::dev_wallet_version as version;

    alloy::sol! {
        /// Kernel meta factory：前端部署账户时 initCode 的调用。
        interface IFactoryStaker {
            function deployWithFactory(address factory, bytes createData, bytes32 salt) external payable returns (address);
        }
    }

    fn key() -> PasskeyPublicKey {
        PasskeyPublicKey::new([0x11; 32], [0x22; 32], b"akari-test-credential")
    }

    /// ZeroDev SDK 5.5.10（`createKernelAccount(...).getFactoryArgs()`，本地开发链第 1 版参数 + 上面的公钥）给出的 `factoryData`：
    /// `deployWithFactory(工厂, initData, salt)`，经 meta factory 部署时作为 initCode。
    const SDK_FACTORY_DATA: &str = "0xc5265d5d0000000000000000000000002577507b78c2008ff367261cb6285d44ba5ef2e90000000000000000000000000000000000000000000000000000000000000060000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002a43c3b752b017ab16ff354acb328452f1d445b3ddee9a91e9e690000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000001200000000000000000000000000000000000000000000000000000000000000140000000000000000000000000000000000000000000000000000000000000006011111111111111111111111111111111111111111111111111111111111111112222222222222222222222222222222222222222222222222222222222222222be58df383d3876315413a6675c262df9268cb210035c0b0de2c95e574ac9a0ea00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000e4e9ae5c530000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000788c313d0ec280837325fa5ab05ac04be31f46abbc0000000000000000000000000000000000000000000000000000000000000000095ea7b30000000000000000000000003dd2fe1ec5cfe2ee0abe62c1e3d81e3d18ee4e1effffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";

    #[test]
    fn init_data_matches_zerodev_sdk() {
        let sdk = IFactoryStaker::deployWithFactoryCall::abi_decode(
            &hex::decode(SDK_FACTORY_DATA).unwrap(),
        )
        .unwrap();
        let v = version();
        assert_eq!(sdk.factory, v.kernel_factory);
        assert_eq!(sdk.salt, ACCOUNT_SALT);
        assert_eq!(sdk.createData, init_data(&v, &key()));
    }

    #[test]
    fn key_paymaster_token_and_validator_change_the_init_data() {
        let (v, k) = (version(), key());
        let base = init_data(&v, &k);
        let other = Address::repeat_byte(0x99);
        let variants = [
            WalletInitVersion {
                paymaster: other,
                ..v.clone()
            },
            WalletInitVersion {
                approve_token: other,
                ..v.clone()
            },
            WalletInitVersion {
                webauthn_validator: other,
                ..v.clone()
            },
            WalletInitVersion {
                approve_amount: U256::from(1),
                ..v.clone()
            },
        ];
        for changed in &variants {
            assert_ne!(init_data(changed, &k), base);
        }
        let other_key = PasskeyPublicKey::new([0x11; 32], [0x22; 32], b"another-credential");
        assert_ne!(init_data(&v, &other_key), base);
    }
}
