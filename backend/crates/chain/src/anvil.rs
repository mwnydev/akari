//! 测试用本机 anvil 与 Kernel v3.3 夹具（`anvil` 特性，只给集成测试用）。

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use alloy::primitives::{U256, address};
use alloy::providers::{DynProvider, Provider, ProviderBuilder};

use crate::ChainError;
use crate::config::WalletInitVersion;

/// Kernel v3.3 工厂（Arbitrum Sepolia `0x2577…F2E9`）的运行时字节码，2026-10-19 用 `cast code` 取得。
/// `getAddress` 只做 CREATE2 推算，不需要 Kernel 实现合约本身，所以只装这一份代码就能在空白 anvil 上推算地址。
const KERNEL_FACTORY_CODE: &str = include_str!("../testdata/kernel-v3.3-factory.hex");

/// 本地 AA 开发链（deploy/dev/aa.sh）的钱包初始化参数第 1 版。
pub fn dev_wallet_version() -> WalletInitVersion {
    WalletInitVersion {
        version: 1,
        kernel_implementation: address!("d6CEDDe84be40893d153Be9d467CD6aD37875b28"),
        kernel_factory: address!("2577507b78c2008Ff367261CB6285d44ba5eF2E9"),
        kernel_meta_factory: address!("d703aaE79538628d27099B8c4f621bE4CCd142d5"),
        webauthn_validator: address!("7ab16Ff354AcB328452F1D445b3Ddee9a91e9e69"),
        entry_point: address!("0000000071727De22E5E9d8BAf0edAc6f37da032"),
        paymaster: address!("6022875397e11A32BbAEB91d80258CBF4AA6F4A8"),
        approve_token: address!("8c313D0eC280837325fA5aB05AC04Be31F46AbBC"),
        approve_amount: U256::MAX,
    }
}

/// 找 anvil：`ANVIL_BIN`、`~/.foundry/bin/anvil` 或 PATH。找不到时调用方跳过测试。
pub fn anvil_bin() -> Option<PathBuf> {
    let candidates = [
        std::env::var_os("ANVIL_BIN").map(PathBuf::from),
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".foundry/bin/anvil")),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|p| p.is_file())
        .or_else(|| {
            Command::new("anvil")
                .arg("--version")
                .stdout(Stdio::null())
                .status()
                .ok()
                .filter(|s| s.success())
                .map(|_| PathBuf::from("anvil"))
        })
}

/// 随机端口上的 anvil 进程，drop 时结束。
pub struct Anvil {
    child: Child,
    port: u16,
}

impl Drop for Anvil {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Anvil {
    pub fn spawn(bin: &PathBuf) -> std::io::Result<Self> {
        let port = TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        let child = Command::new(bin)
            .args(["--port", &port.to_string(), "--silent"])
            .stdout(Stdio::null())
            .spawn()?;
        Ok(Self { child, port })
    }

    pub fn http(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn ws(&self) -> String {
        format!("ws://127.0.0.1:{}", self.port)
    }

    /// HTTP 客户端，等到节点能响应才返回（不在客户端填 nonce / gas：anvil 的 eth_sendTransaction 会自己填，
    /// `evm_revert` 之后也不会用到过期的 nonce）。
    pub async fn provider(&self) -> Result<DynProvider, ChainError> {
        let url = self
            .http()
            .parse()
            .map_err(|e| ChainError::Config(format!("anvil url: {e}")))?;
        let p = ProviderBuilder::new()
            .disable_recommended_fillers()
            .connect_http(url)
            .erased();
        let mut last = ChainError::Rpc("anvil not started".into());
        for _ in 0..100 {
            match p.get_block_number().await {
                Ok(_) => return Ok(p),
                Err(e) => last = e.into(),
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        Err(last)
    }
}

/// 把 Kernel v3.3 工厂装到 `version.kernel_factory`（`anvil_setCode`）。
pub async fn install_kernel_factory(
    p: &DynProvider,
    version: &WalletInitVersion,
) -> Result<(), ChainError> {
    let _: serde_json::Value = p
        .raw_request(
            "anvil_setCode".into(),
            (version.kernel_factory, KERNEL_FACTORY_CODE.trim()),
        )
        .await?;
    Ok(())
}
