//! 拍卖相关的链上读取：出价资格的钱包 USDT 余额、某场拍卖是否已在链上成交（`Escrow.auctionSettled`）。

use alloy::primitives::{Address, B256, U256, keccak256};
use alloy::providers::Provider;
use alloy::rpc::types::{TransactionInput, TransactionRequest};
use alloy::sol_types::{SolCall, SolValue};

use crate::ChainError;
use crate::bindings::Escrow;
use crate::rpc::FailoverRpc;

alloy::sol! {
    interface IERC20Balance {
        function balanceOf(address owner) external view returns (uint256);
    }
}

/// `Escrow.auctionSettled` 的键：`keccak256(abi.encode(seller, auctionId))`。
pub fn settled_key(seller: Address, auction_id: U256) -> B256 {
    keccak256((seller, auction_id).abi_encode())
}

async fn call<C: SolCall>(
    rpc: &FailoverRpc,
    what: &str,
    to: Address,
    c: C,
) -> Result<C::Return, ChainError> {
    let data = c.abi_encode();
    rpc.call(what, |p| {
        let tx = TransactionRequest::default()
            .to(to)
            .input(TransactionInput::new(data.clone().into()));
        async move {
            let out = p.call(tx).await?;
            C::abi_decode_returns(&out).map_err(|e| ChainError::Decode(format!("{what}: {e}")))
        }
    })
    .await
}

/// 钱包的 USDT 余额（latest，最小单位）。
pub async fn token_balance(
    rpc: &FailoverRpc,
    token: Address,
    owner: Address,
) -> Result<U256, ChainError> {
    call(
        rpc,
        "eth_call balanceOf",
        token,
        IERC20Balance::balanceOfCall { owner },
    )
    .await
}

/// 这场拍卖是否已在链上成交（latest）。
pub async fn auction_settled(
    rpc: &FailoverRpc,
    escrow: Address,
    seller: Address,
    auction_id: U256,
) -> Result<bool, ChainError> {
    call(
        rpc,
        "eth_call auctionSettled",
        escrow,
        Escrow::auctionSettledCall(settled_key(seller, auction_id)),
    )
    .await
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn settled_key_is_abi_encode_of_seller_and_id() {
        let seller = Address::repeat_byte(0x11);
        let mut buf = [0u8; 64];
        buf[12..32].copy_from_slice(seller.as_slice());
        buf[63] = 42;
        assert_eq!(settled_key(seller, U256::from(42)), keccak256(buf));
    }
}
