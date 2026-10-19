//! 余额巡检（plan 第 9 节）：Paymaster 押金低于 0.05 ETH、热钱包低于 0.01 ETH 时发管理员群告警
//! （[`outbox::staff_alert_deduped`]，同一种 `dedupe` 内只发一次），由站长补充（不做自动兑换）。
//! 热钱包余额不足时代发器同时视为不健康。

use alloy::primitives::utils::format_ether;
use alloy::primitives::{Address, U256};
use forum_db::PgPool;
use forum_db::outbox;
use tokio_util::sync::CancellationToken;

use crate::chain::Chain;
use crate::config::AlertConfig;
use crate::relayer::Relayer;

pub const PAYMASTER_DEPOSIT_LOW: &str = "sponsor.paymaster_deposit_low";
pub const HOT_WALLET_LOW: &str = "sponsor.hot_wallet_low";

async fn alert(
    db: &PgPool,
    cfg: &AlertConfig,
    event: &str,
    what: &str,
    address: Address,
    balance: U256,
    threshold: U256,
) -> anyhow::Result<()> {
    let text = format!(
        "{what} {address} 余额 {} ETH，低于告警线 {} ETH，请补充",
        format_ether(balance),
        format_ether(threshold)
    );
    if outbox::staff_alert_deduped(db, event, &text, cfg.dedupe.as_secs_f64()).await? {
        tracing::warn!(event, %address, %balance, "balance alert raised");
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
            "Paymaster 在 EntryPoint 的押金",
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
        alert(
            db,
            cfg,
            HOT_WALLET_LOW,
            "代发热钱包",
            hot,
            balance,
            cfg.hot_wallet_min,
        )
        .await?;
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
