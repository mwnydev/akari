//! forum-sponsor：代付签名 + 私有代发（plan 第 5 节「gas 规则」，ADR 0003、0004）。
//!
//! 唯一持有 Paymaster 签名私钥和代发热钱包私钥的程序。forum-api 校验会话、来源与限频后，把钱包页面的
//! JSON-RPC 请求连同用户 ID 转发到这里（[`server`]）；RPC 与 bundler 密钥从不到浏览器。

pub mod alerts;
pub mod bundler;
pub mod chain;
pub mod config;
pub mod jsonrpc;
pub mod kernel;
pub mod keys;
pub mod ledger;
pub mod policy;
pub mod relayer;
pub mod server;
pub mod sim;
pub mod sponsor;

use std::sync::Arc;

use alloy::primitives::B256;
use alloy::signers::local::PrivateKeySigner;
use alloy::sol_types::SolCall;
use anyhow::{Context, bail};
use forum_chain::bindings::FixedFeePaymaster;
use forum_chain::rpc::FailoverRpc;
use forum_db::PgPool;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::bundler::{Bundler, Fallback};
use crate::chain::{Chain, call_request};
use crate::config::Config;
use crate::relayer::Relayer;
use crate::sponsor::Sponsor;

/// 组装好的服务。
pub struct App {
    pub cfg: Config,
    pub db: PgPool,
    pub chain: Arc<Chain>,
    pub bundler: Arc<Bundler>,
}

impl App {
    pub fn new(
        cfg: Config,
        db: PgPool,
        paymaster_signer: PrivateKeySigner,
        hot_wallet: PrivateKeySigner,
    ) -> anyhow::Result<Self> {
        let chain = Arc::new(Chain {
            rpc: FailoverRpc::from_config(&cfg.chain)?,
            chain_id: cfg.chain.chain_id,
            entry_point: cfg.entry_point(),
        });
        let relayer = Relayer::new(db.clone(), chain.clone(), hot_wallet, cfg.relayer.clone());
        let sponsor = Sponsor::new(
            db.clone(),
            chain.clone(),
            cfg.contracts(),
            cfg.aa.clone(),
            cfg.policy.clone(),
            paymaster_signer,
            relayer.address(),
        );
        let fallback = cfg
            .fallback_bundler
            .as_deref()
            .map(Fallback::new)
            .transpose()?;
        let bundler = Arc::new(Bundler {
            sponsor: Arc::new(sponsor),
            relayer,
            fallback,
        });
        Ok(Self {
            cfg,
            db,
            chain,
            bundler,
        })
    }

    /// 启动前核对链上配置：Paymaster 的 `signer` 必须是本服务的签名私钥，否则签出的代付全部无效。
    pub async fn verify_onchain(&self) -> anyhow::Result<()> {
        let pm = self.cfg.aa.paymaster;
        let tx = call_request(
            alloy::primitives::Address::ZERO,
            pm,
            FixedFeePaymaster::signerCall {}.abi_encode().into(),
        );
        let out = self
            .chain
            .call(&tx)
            .await?
            .map_err(|e| anyhow::anyhow!("paymaster signer(): {e}"))?;
        let onchain = FixedFeePaymaster::signerCall::abi_decode_returns(&out)
            .context("paymaster signer() returned garbage")?;
        let ours = self.bundler.sponsor.signer_address();
        if onchain != ours {
            bail!("paymaster signer is {onchain}, but the loaded key is {ours}");
        }
        // 备用模拟靠覆盖这个存储槽换掉签名者，槽位不对时模拟会全部失败（不会放行），这里提前报错。
        let slot = self
            .chain
            .storage_at(pm, sim::PAYMASTER_SIGNER_SLOT)
            .await?;
        if B256::from(slot) != onchain.into_word() {
            bail!(
                "paymaster signer is not in storage slot {}",
                sim::PAYMASTER_SIGNER_SLOT
            );
        }
        if self.chain.code_len(self.cfg.entry_point()).await? == 0 {
            bail!("no EntryPoint code at {}", self.cfg.entry_point());
        }
        Ok(())
    }

    pub fn router(&self) -> axum::Router {
        server::router(self.bundler.clone(), &self.cfg.api_token)
    }

    /// 代发器、过期对账与余额巡检。
    pub fn spawn_background(&self, shutdown: &CancellationToken) -> Vec<JoinHandle<()>> {
        let relayer = self.bundler.relayer.clone();
        vec![
            tokio::spawn(relayer.clone().run(shutdown.clone())),
            tokio::spawn(ledger::run_reconciler(
                self.db.clone(),
                self.chain.clone(),
                self.cfg.chain.indexer.max_block_range,
                shutdown.clone(),
            )),
            tokio::spawn(alerts::run(
                self.db.clone(),
                self.chain.clone(),
                self.cfg.aa.paymaster,
                relayer,
                self.cfg.alerts.clone(),
                shutdown.clone(),
            )),
        ]
    }
}
