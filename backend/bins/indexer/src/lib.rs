//! forum-indexer：链上状态进入论坛数据库的唯一入口（plan 第 2、8 节）。
//!
//! WebSocket 订阅合约日志即时写入（latest），每 `reconcile_interval_ms`（默认 5 秒）按区块范围批量补拉校对、
//! 处理区块回滚，每隔几轮按 RPC 的 safe / finalized 标记升级确认级别。订单每次变化写一条
//! `order.updated` 发件箱消息，由 worker 投递；订单最终确认后在同一事务里更新双方交易信誉与经验。

pub mod chat;
pub mod engine;
pub mod rpc;
pub mod ws;

use std::time::Duration;

use forum_chain::config::ChainConfig;
use forum_db::PgPool;
use sqlx::Connection;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub use engine::{Engine, IndexerError};
pub use rpc::RpcSource;

/// 启动时等待其他 indexer 释放锁的最长时间。
const LOCK_WAIT: Duration = Duration::from_secs(10);
/// WebSocket 日志缓冲；满了会让订阅任务等待，不会丢（丢了也由校对补上）。
const LIVE_BUFFER: usize = 1024;

/// 运行到 `shutdown`。同一条链已有 indexer 在运行（等待 [`LOCK_WAIT`] 仍未释放）时返回 [`IndexerError::Locked`]。
/// `cache` 用于订单最终确认后失效用户名片，以及拍卖成交或回滚后失效游客缓存里的列表（同一个 Valkey）。
pub async fn run(
    pool: PgPool,
    cfg: ChainConfig,
    cache: Option<forum_kv::ForumCache>,
    shutdown: CancellationToken,
) -> Result<(), IndexerError> {
    let chain_id = i64::try_from(cfg.chain_id)
        .map_err(|_| forum_chain::ChainError::Config("chain_id exceeds i64".into()))?;
    let Some(lock) = lock(&pool, chain_id, &shutdown).await? else {
        return Err(IndexerError::Locked(cfg.chain_id));
    };
    let src = RpcSource::new(&cfg)?;
    let mut engine = Engine::open(pool.clone(), &cfg, cache).await?;
    tracing::info!(chain_id, cursor = ?engine.progress().cursor.map(|c| c.number), "indexer started");

    let (tx, mut live) = mpsc::channel(LIVE_BUFFER);
    let ws_urls = cfg
        .endpoints()
        .iter()
        .filter_map(|e| e.ws.clone())
        .collect();
    let ws = tokio::spawn(ws::run(
        ws_urls,
        cfg.indexed_contracts(),
        tx,
        shutdown.child_token(),
    ));

    let mut ticker = tokio::time::interval(cfg.indexer.reconcile_interval());
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut rounds: u64 = 0;
    let mut failovers = 0;
    loop {
        tokio::select! {
            biased;
            () = shutdown.cancelled() => break,
            _ = ticker.tick() => {
                reconcile(&mut engine, &src, &shutdown, rounds.is_multiple_of(u64::from(cfg.indexer.promote_every))).await;
                rounds += 1;
                failovers = alert_failover(&pool, &src, chain_id, failovers).await;
            }
            Some(log) = live.recv() => match engine.ingest_live(&src, &log).await {
                Ok(false) => {}
                Ok(true) => {
                    tracing::info!(block = log.block.number, "removed log pushed, reconciling now");
                    reconcile(&mut engine, &src, &shutdown, false).await;
                }
                Err(e) => tracing::warn!(error = %e, "live log not stored; reconcile will retry"),
            },
        }
    }
    live.close();
    if let Err(e) = ws.await {
        tracing::warn!(error = %e, "websocket task panicked");
    }
    if let Err(e) = lock.close().await {
        tracing::warn!(error = %e, "closing lock connection");
    }
    tracing::info!(chain_id, "indexer stopped");
    Ok(())
}

/// 管理员群告警：切到备用 RPC 节点（主节点故障，备用通常更贵或限额更低）。同一条链 1 小时内只推送一次。
pub const RPC_FAILOVER_ALERT: &str = "indexer.rpc_failover";
const RPC_FAILOVER_DEDUPE_SECS: f64 = 3600.0;

/// 自上次检查以来切过备用节点就发告警，返回已告警到的累计次数。告警写不进去时下一轮重试。
async fn alert_failover(pool: &PgPool, src: &RpcSource, chain_id: i64, seen: u64) -> u64 {
    let now = src.failovers();
    if now <= seen {
        return now;
    }
    let event = format!("{RPC_FAILOVER_ALERT}:{chain_id}");
    let text = format!(
        "链 {chain_id} 的 indexer 已切到备用 RPC 节点 {}（主节点故障），请检查主节点",
        src.active_name()
    );
    match forum_db::outbox::staff_alert_deduped(pool, &event, &text, RPC_FAILOVER_DEDUPE_SECS).await
    {
        Ok(_) => now,
        Err(e) => {
            tracing::warn!(error = %e, "rpc failover alert not written, retrying next round");
            seen
        }
    }
}

/// 取得本链的 indexer 锁。锁绑定在连接上：从连接池摘下来单独持有，退出时关闭即释放。
/// 刚退出的进程的数据库会话可能还没结束，所以重试 [`LOCK_WAIT`] 再放弃。
async fn lock(
    pool: &PgPool,
    chain_id: i64,
    shutdown: &CancellationToken,
) -> Result<Option<sqlx::PgConnection>, IndexerError> {
    let deadline = tokio::time::Instant::now() + LOCK_WAIT;
    let mut conn = pool.acquire().await?.detach();
    loop {
        if forum_db::chain::try_lock_indexer(&mut conn, chain_id).await? {
            return Ok(Some(conn));
        }
        if tokio::time::Instant::now() >= deadline || shutdown.is_cancelled() {
            return Ok(None);
        }
        tracing::info!(chain_id, "another indexer holds the chain lock, waiting");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// 校对直到追上最新块；出错只记录，下个周期重试（RPC 已在内部切换节点）。
async fn reconcile(
    engine: &mut Engine,
    src: &RpcSource,
    shutdown: &CancellationToken,
    promote: bool,
) {
    loop {
        let plan = match engine.reconcile(src).await {
            Ok((plan, applied)) => {
                if applied != engine::Applied::default() || plan.rollback_after.is_some() {
                    tracing::info!(
                        to = plan.cursor.map(|c| c.number),
                        head = plan.head.number,
                        rollback_after = plan.rollback_after,
                        inserted = applied.inserted,
                        removed = applied.removed,
                        orders = applied.orders_changed,
                        "reconciled"
                    );
                }
                plan
            }
            Err(e) => {
                tracing::warn!(error = %e, "reconcile failed");
                return;
            }
        };
        if !plan.behind() || shutdown.is_cancelled() {
            if promote && let Err(e) = engine.promote(src, plan.head.number).await {
                tracing::warn!(error = %e, "confirmation update failed");
            }
            return;
        }
    }
}
