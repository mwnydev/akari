//! 链上结果入账与过期对账。
//!
//! - 代发器拿到回执后、对账任务在日志里找到 `UserOperationEvent` 后，都走 [`record`]：写结果，callData 失败的
//!   计一次链上失败（plan 第 9 节：同账号 3 次暂停代付 24 小时）。同一笔只会入账一次。
//! - 签名过期后结果就确定了（EntryPoint 不再接受它）：对账任务从签名时的区块起查日志，找到就入账，
//!   找不到记为过期，免费额度随之释放。经第三方 bundler 或被用户拿去别处提交的操作也这样入账。

use std::time::Duration;

use alloy::primitives::{Address, B256, U256};
use alloy::rpc::types::{Filter, Log};
use alloy::sol_types::SolEvent;
use forum_chain::aa::IEntryPoint;
use forum_db::PgPool;
use forum_db::sponsor::{self as db, Outcome, SponsorshipStatus};
use tokio_util::sync::CancellationToken;

use crate::chain::Chain;

/// 链上失败几次暂停代付。
pub const FAILURE_LIMIT: i32 = 3;
/// 失败计数窗口与暂停时长：24 小时。
pub const FAILURE_WINDOW_SECS: f64 = 86_400.0;
pub const SUSPEND_SECS: f64 = 86_400.0;
/// 签名过期多久后对账（给最后一个区块的日志留出时间）。
const RECONCILE_GRACE_SECS: f64 = 30.0;
const RECONCILE_BATCH: i64 = 100;
const RECONCILE_INTERVAL: Duration = Duration::from_secs(20);

/// 一笔 UserOperation 的链上结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpOutcome {
    pub user_op_hash: B256,
    pub success: bool,
    pub tx_hash: B256,
    pub block_number: u64,
    pub actual_gas_cost: U256,
}

/// 从日志里取出 EntryPoint 的 `UserOperationEvent`。
pub fn outcomes(logs: &[Log], entry_point: Address) -> Vec<OpOutcome> {
    logs.iter()
        .filter(|l| l.address() == entry_point)
        .filter_map(|l| {
            let e = IEntryPoint::UserOperationEvent::decode_log(&l.inner).ok()?;
            Some(OpOutcome {
                user_op_hash: e.userOpHash,
                success: e.success,
                tx_hash: l.transaction_hash?,
                block_number: l.block_number?,
                actual_gas_cost: e.actualGasCost,
            })
        })
        .collect()
}

/// 入账一笔结果；callData 失败时累计失败次数，达到上限就暂停该用户的代付。
pub async fn record(db: &PgPool, o: &OpOutcome) -> anyhow::Result<()> {
    let mut tx = db.begin().await?;
    let cost = o.actual_gas_cost.to_string();
    let failed_user = db::settle(
        &mut *tx,
        Outcome {
            user_op_hash: o.user_op_hash.as_slice(),
            success: o.success,
            tx_hash: o.tx_hash.as_slice(),
            block_number: i64::try_from(o.block_number)?,
            actual_gas_cost: &cost,
        },
    )
    .await?;
    if let Some(user_id) = failed_user {
        let suspended = db::record_failure(
            &mut tx,
            user_id,
            FAILURE_LIMIT,
            FAILURE_WINDOW_SECS,
            SUSPEND_SECS,
        )
        .await?;
        tracing::warn!(user_id, user_op_hash = %o.user_op_hash, "sponsored operation failed on chain");
        if let Some(until) = suspended {
            tracing::warn!(user_id, %until, "sponsorship suspended after repeated failures");
        }
    }
    tx.commit().await?;
    Ok(())
}

/// 处理一批到期记录，返回处理条数。
pub async fn reconcile_once(db: &PgPool, chain: &Chain, max_range: u64) -> anyhow::Result<usize> {
    let due = db::due_for_reconcile(db, RECONCILE_GRACE_SECS, RECONCILE_BATCH).await?;
    let Some(from) = due.iter().map(|d| d.signed_block).min() else {
        return Ok(0);
    };
    let hashes: Vec<B256> = due
        .iter()
        .filter_map(|d| B256::try_from(d.user_op_hash.as_slice()).ok())
        .collect();
    let latest = chain.block_number().await?;
    let mut found = Vec::new();
    let mut start = u64::try_from(from)?;
    while start <= latest {
        let end = latest.min(start + max_range.max(1) - 1);
        let filter = Filter::new()
            .address(chain.entry_point)
            .event_signature(IEntryPoint::UserOperationEvent::SIGNATURE_HASH)
            .topic1(hashes.clone())
            .from_block(start)
            .to_block(end);
        found.extend(outcomes(&chain.logs(&filter).await?, chain.entry_point));
        start = end + 1;
    }
    for o in &found {
        record(db, o).await?;
    }
    for h in &hashes {
        if !found.iter().any(|o| o.user_op_hash == *h) {
            db::close(db, h.as_slice(), SponsorshipStatus::Expired).await?;
        }
    }
    Ok(due.len())
}

pub async fn run_reconciler(
    db: PgPool,
    chain: std::sync::Arc<Chain>,
    max_range: u64,
    shutdown: CancellationToken,
) {
    loop {
        match reconcile_once(&db, &chain, max_range).await {
            Ok(n) if n > 0 => tracing::info!(n, "reconciled expired sponsorships"),
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "reconcile failed"),
        }
        tokio::select! {
            () = shutdown.cancelled() => return,
            () = tokio::time::sleep(RECONCILE_INTERVAL) => {}
        }
    }
}
