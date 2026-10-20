//! 链下验签（`forum_chain::sig`）在本机 anvil 上：EOA 的 ECDSA 签名、签错的人、乱码签名、没有 ERC-1271 的合约。
//! ERC-6492（未部署的智能账户）见 forum-api 的 `auction_e2e.rs`。找不到 anvil 时跳过。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

use alloy::primitives::{Address, B256, keccak256};
use alloy::providers::Provider;
use alloy::signers::SignerSync;
use alloy::signers::local::PrivateKeySigner;
use forum_chain::anvil::{Anvil, anvil_bin};
use forum_chain::bindings::MockUSDT;
use forum_chain::sig::signature_valid;

#[tokio::test]
async fn checks_signatures_like_the_contract() {
    let Some(bin) = anvil_bin() else {
        eprintln!("skipped: anvil not found (install Foundry or set ANVIL_BIN)");
        return;
    };
    let anvil = Anvil::spawn(&bin).unwrap();
    let p = anvil.provider().await.unwrap();
    let hash: B256 = keccak256(b"auction authorization");
    let signer = PrivateKeySigner::random();
    let sig = signer.sign_hash_sync(&hash).unwrap().as_bytes();
    assert!(
        signature_valid(&p, signer.address(), hash, &sig)
            .await
            .unwrap()
    );
    // 别人签的、签的是别的摘要、乱码
    let other = PrivateKeySigner::random()
        .sign_hash_sync(&hash)
        .unwrap()
        .as_bytes();
    assert!(
        !signature_valid(&p, signer.address(), hash, &other)
            .await
            .unwrap()
    );
    let wrong_hash = keccak256(b"something else");
    assert!(
        !signature_valid(&p, signer.address(), wrong_hash, &sig)
            .await
            .unwrap()
    );
    assert!(
        !signature_valid(&p, signer.address(), hash, &[1, 2, 3])
            .await
            .unwrap()
    );
    // 有代码但没有 isValidSignature 的地址：无效（不会退回 ECDSA）
    let deployer = p.get_accounts().await.unwrap()[0];
    let token = MockUSDT::deploy_builder(&p)
        .from(deployer)
        .deploy()
        .await
        .unwrap();
    assert!(!signature_valid(&p, token, hash, &sig).await.unwrap());
    assert!(
        !signature_valid(&p, Address::ZERO, hash, &sig)
            .await
            .unwrap()
    );
}
