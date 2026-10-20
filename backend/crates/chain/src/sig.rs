//! 链下验签（拍卖授权）：与 `Escrow` 用同一套规则判断卖家签名是否有效——EOA 走 ECDSA，已部署的智能账户走
//! ERC-1271，未部署的智能账户走 ERC-6492（经链上 Solady 验证器临时部署，结果回滚）。
//!
//! 做法是 ERC-6492 的「无部署」调用：`eth_call` 不带 `to`，数据为 `sig/SignatureCheck.sol` 的创建字节码 + 构造参数，
//! 构造函数直接返回 32 字节的结果。验证器合约不需要部署，只读、不花 gas。

use alloy::primitives::{Address, B256, Bytes, U256};
use alloy::providers::{DynProvider, Provider};
use alloy::rpc::types::{TransactionInput, TransactionRequest};
use alloy::sol_types::SolValue;

use crate::ChainError;
use crate::rpc::FailoverRpc;

/// `sig/SignatureCheck.sol` 的创建字节码（`sig/regen.sh` 生成，`tests/sig_bytecode.rs` 校验没有过期）。
const SIGNATURE_CHECK_CODE: &str = include_str!("../sig/SignatureCheck.hex");

fn creation_code(signer: Address, hash: B256, signature: &[u8]) -> Result<Bytes, ChainError> {
    let code: Bytes = SIGNATURE_CHECK_CODE
        .trim()
        .parse()
        .map_err(|e| ChainError::Config(format!("SignatureCheck bytecode: {e}")))?;
    let args = (signer, hash, Bytes::copy_from_slice(signature)).abi_encode_params();
    Ok([code.as_ref(), args.as_slice()].concat().into())
}

/// 节点报的是合约执行失败（不是节点故障）：按无效签名处理，不切换节点。
fn is_revert(e: &alloy::transports::TransportError) -> bool {
    e.as_error_resp()
        .is_some_and(|p| p.code == 3 || p.message.to_ascii_lowercase().contains("revert"))
}

/// 在一个节点上判断 `signature` 是不是 `signer` 对 `hash` 的有效签名（latest 区块）。
pub async fn signature_valid(
    provider: &DynProvider,
    signer: Address,
    hash: B256,
    signature: &[u8],
) -> Result<bool, ChainError> {
    let tx = TransactionRequest {
        input: TransactionInput::new(creation_code(signer, hash, signature)?),
        ..Default::default()
    };
    match provider.call(tx).await {
        Ok(out) => {
            let v = U256::abi_decode(&out)
                .map_err(|e| ChainError::Decode(format!("SignatureCheck result: {e}")))?;
            Ok(v == U256::from(1))
        }
        Err(e) if is_revert(&e) => Ok(false),
        Err(e) => Err(e.into()),
    }
}

/// 同 [`signature_valid`]，按主备节点依次尝试。
pub async fn signature_valid_on(
    rpc: &FailoverRpc,
    signer: Address,
    hash: B256,
    signature: &[u8],
) -> Result<bool, ChainError> {
    rpc.call("eth_call SignatureCheck", |p| async move {
        signature_valid(&p, signer, hash, signature).await
    })
    .await
}
