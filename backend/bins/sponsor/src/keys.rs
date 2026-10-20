//! 私钥加载（plan 第 11 节密钥表）：生产由 systemd `LoadCredentialEncrypted` 解密到
//! `$CREDENTIALS_DIRECTORY/<名字>`；本地开发没有该目录时读环境变量。私钥从不写日志、不出现在错误信息里。

use std::path::Path;

use alloy::signers::local::PrivateKeySigner;
use anyhow::{Context, bail};

/// Paymaster 签名私钥（只能批准代付）。
pub const PAYMASTER_SIGNER: Credential = Credential {
    name: "paymaster-signer",
    dev_env: "SPONSOR_DEV_PAYMASTER_KEY",
};
/// 代发热钱包私钥（提交 handleOps，只放少量 ETH）。
pub const HOT_WALLET: Credential = Credential {
    name: "relayer-hot-wallet",
    dev_env: "SPONSOR_DEV_HOT_WALLET_KEY",
};

/// 拍卖成交证明签名私钥（`EscrowConfig.auctionSigner`；只能签成交证明，见 [`crate::auction`]）。可选：没有时不签证明，
/// 拍卖无法成交（`createFromAuction` 也不会被代付）。
pub const AUCTION_SIGNER: Credential = Credential {
    name: "auction-signer",
    dev_env: "SPONSOR_DEV_AUCTION_KEY",
};

/// 生产链：这里绝不接受环境变量里的开发密钥。
const MAINNETS: [u64; 1] = [42_161];

#[derive(Debug, Clone, Copy)]
pub struct Credential {
    /// systemd 凭据名（deploy/ansible `app_services[].credentials`）。
    pub name: &'static str,
    pub dev_env: &'static str,
}

fn parse(text: &str, what: &str) -> anyhow::Result<PrivateKeySigner> {
    // 不用 `?` 透传解析错误：错误信息可能带出部分输入。
    text.trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("{what}: not a valid secp256k1 private key"))
}

impl Credential {
    pub fn load(&self, chain_id: u64) -> anyhow::Result<PrivateKeySigner> {
        self.load_from(
            std::env::var_os("CREDENTIALS_DIRECTORY")
                .as_deref()
                .map(Path::new),
            std::env::var(self.dev_env).ok(),
            chain_id,
        )
    }

    /// 同 [`Self::load`]，但凭据与开发变量都没有时返回空（可选的私钥）。
    pub fn load_optional(&self, chain_id: u64) -> anyhow::Result<Option<PrivateKeySigner>> {
        self.load_optional_from(
            std::env::var_os("CREDENTIALS_DIRECTORY")
                .as_deref()
                .map(Path::new),
            std::env::var(self.dev_env).ok(),
            chain_id,
        )
    }

    fn load_optional_from(
        &self,
        dir: Option<&Path>,
        dev: Option<String>,
        chain_id: u64,
    ) -> anyhow::Result<Option<PrivateKeySigner>> {
        let dev = dev.filter(|v| !v.is_empty());
        if dev.is_none() && !dir.is_some_and(|d| d.join(self.name).exists()) {
            return Ok(None);
        }
        self.load_from(dir, dev, chain_id).map(Some)
    }

    fn load_from(
        &self,
        dir: Option<&Path>,
        dev: Option<String>,
        chain_id: u64,
    ) -> anyhow::Result<PrivateKeySigner> {
        if let Some(dir) = dir {
            let path = dir.join(self.name);
            if path.exists() {
                let text = std::fs::read_to_string(&path)
                    .with_context(|| format!("read credential {}", self.name))?;
                return parse(&text, self.name);
            }
        }
        let Some(dev) = dev.filter(|v| !v.is_empty()) else {
            bail!(
                "credential {} not found (set {} for local development)",
                self.name,
                self.dev_env
            );
        };
        if MAINNETS.contains(&chain_id) {
            bail!(
                "{} is a development fallback and is refused on chain {chain_id}",
                self.dev_env
            );
        }
        tracing::warn!(
            credential = self.name,
            "using development key from environment"
        );
        parse(&dev, self.dev_env)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    /// anvil 默认助记词第 0 号账户（公开）。
    const KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

    #[test]
    fn credentials_directory_wins_over_env() {
        let dir = std::env::temp_dir().join(format!("sponsor-keys-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(PAYMASTER_SIGNER.name), format!("{KEY}\n")).unwrap();
        let s = PAYMASTER_SIGNER
            .load_from(Some(&dir), Some("garbage".into()), 42_161)
            .unwrap();
        assert_eq!(
            s.address().to_string(),
            "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn dev_fallback_only_off_mainnet() {
        assert!(HOT_WALLET.load_from(None, Some(KEY.into()), 31_337).is_ok());
        assert!(
            HOT_WALLET
                .load_from(None, Some(KEY.into()), 42_161)
                .is_err()
        );
        assert!(HOT_WALLET.load_from(None, None, 31_337).is_err());
    }

    #[test]
    fn auction_signer_is_optional_but_still_guarded() {
        assert!(
            AUCTION_SIGNER
                .load_optional_from(None, None, 42_161)
                .unwrap()
                .is_none()
        );
        assert!(
            AUCTION_SIGNER
                .load_optional_from(None, Some(String::new()), 31_337)
                .unwrap()
                .is_none()
        );
        assert!(
            AUCTION_SIGNER
                .load_optional_from(None, Some(KEY.into()), 31_337)
                .unwrap()
                .is_some()
        );
        // 主网上开发私钥照样拒绝
        assert!(
            AUCTION_SIGNER
                .load_optional_from(None, Some(KEY.into()), 42_161)
                .is_err()
        );
    }

    #[test]
    fn errors_never_echo_the_key() {
        let bad = "0xdeadbeefsecret";
        let err = HOT_WALLET
            .load_from(None, Some(bad.into()), 31_337)
            .unwrap_err();
        assert!(!format!("{err:#}").contains("deadbeef"));
        let signer = HOT_WALLET
            .load_from(None, Some(KEY.into()), 31_337)
            .unwrap();
        assert!(!format!("{signer:?}").contains(&KEY[2..10]));
    }
}
