//! 余额巡检（plan 第 9 节）：Paymaster 押金低于 0.05 ETH、热钱包低于 0.01 ETH 时写告警到发件箱，
//! 由 worker 通知站长补充（不做自动兑换）。热钱包余额不足时代发器同时视为不健康。

use alloy::primitives::{Address, U256};
use forum_db::PgPool;
use forum_db::sponsor as db;
use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::chain::Chain;
use crate::config::AlertConfig;
use crate::relayer::Relayer;

pub const PAYMASTER_DEPOSIT_LOW: &str = "alert.paymaster_deposit_low";
pub const HOT_WALLET_LOW: &str = "alert.hot_wallet_low";

async fn alert(
    db: &PgPool,
    cfg: &AlertConfig,
    kind: &str,
    address: Address,
    balance: U256,
    threshold: U256,
) -> anyhow::Result<()> {
    let payload = json!({
        "address": address,
        "balance_wei": balance.to_string(),
        "threshold_wei": threshold.to_string(),
    });
    if db::enqueue_alert(db, kind, &payload, cfg.dedupe.as_secs_f64()).await? {
        tracing::warn!(kind, %address, %balance, "balance alert raised");
    }
    Ok(())
}

pub async fn check_once(
    db: &PgPool,
    chain: &Chain,
    paymaster: Address,
    relayer: &Relayer,
    cfg: &AlertConfig,
) -> anyhow::Result<()> {
    let deposit = chain.deposit_of(paymaster).await?;
    if deposit < cfg.deposit_min {
        alert(
            db,
            cfg,
            PAYMASTER_DEPOSIT_LOW,
            paymaster,
            deposit,
            cfg.deposit_min,
        )
        .await?;
    }
    let hot = relayer.address();
    let balance = chain.balance(hot).await?;
    relayer.set_balance_ok(balance >= cfg.hot_wallet_min);
    if balance < cfg.hot_wallet_min {
        alert(db, cfg, HOT_WALLET_LOW, hot, balance, cfg.hot_wallet_min).await?;
    }
    Ok(())
}

pub async fn run(
    db: PgPool,
    chain: std::sync::Arc<Chain>,
    paymaster: Address,
    relayer: Relayer,
    cfg: AlertConfig,
    shutdown: CancellationToken,
) {
    loop {
        if let Err(e) = check_once(&db, &chain, paymaster, &relayer, &cfg).await {
            tracing::warn!(error = %e, "balance check failed");
        }
        tokio::select! {
            () = shutdown.cancelled() => return,
            () = tokio::time::sleep(cfg.interval) => {}
        }
    }
}
