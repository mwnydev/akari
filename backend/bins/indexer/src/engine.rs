//! 把链上数据写进数据库：执行校对计划、写入 WebSocket 即时日志、升级确认级别，并在订单变化时写发件箱。
//!
//! 所有写入都由一个任务串行执行（见 [`crate::run`]），并且同一条链只允许一个 indexer 进程（会话级咨询锁）。
//! 因此读库、调 RPC、再开事务写库之间不会有别的写入者插进来。

use std::collections::BTreeMap;

use alloy::primitives::{Address, B256, U256};
use forum_chain::ChainError;
use forum_chain::config::ChainConfig;
use forum_chain::event::{self, BlockRef, LogKey, RawLog};
use forum_chain::order::{self, Level, OrderLog, OrderView};
use forum_chain::reorg::{self, ChainSource, Plan, Progress, Stored};
use forum_chain::snapshot::OrderSnapshot;
use forum_db::chain::{self as db, OrderRef};
use forum_db::{PgPool, Postgres, Transaction, outbox};
use serde_json::{Value, json};
use time::OffsetDateTime;

/// 发件箱消息类型：订单状态（含确认级别）每次变化一条。
pub const ORDER_UPDATED: &str = "order.updated";

#[derive(Debug, thiserror::Error)]
pub enum IndexerError {
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error(transparent)]
    Chain(#[from] ChainError),
    #[error("another indexer holds the lock for chain {0}")]
    Locked(u64),
}

/// 订单变化的原因，写进 `order.updated` 的 `cause`；`reorg` 即「状态修正」通知。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Cause {
    Confirmation,
    Event,
    Reorg,
}

impl Cause {
    fn as_str(self) -> &'static str {
        match self {
            Self::Confirmation => "confirmation",
            Self::Event => "event",
            Self::Reorg => "reorg",
        }
    }
}

/// 一轮执行的统计（日志与测试用）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Applied {
    pub inserted: usize,
    pub removed: usize,
    pub orders_changed: usize,
}

pub struct Engine {
    pool: PgPool,
    chain_id: i64,
    start_block: u64,
    max_range: u64,
    policy: forum_chain::config::ConfirmationPolicy,
    progress: Progress,
}

fn db_i64(v: u64) -> Result<i64, IndexerError> {
    i64::try_from(v).map_err(|_| ChainError::Decode(format!("{v} exceeds i64")).into())
}

fn db_u64(v: i64) -> Result<u64, IndexerError> {
    u64::try_from(v).map_err(|_| ChainError::Decode(format!("negative {v}")).into())
}

fn db_i32(v: u64) -> Result<i32, IndexerError> {
    i32::try_from(v).map_err(|_| ChainError::Decode(format!("{v} exceeds i32")).into())
}

fn b256(bytes: &[u8]) -> Result<B256, IndexerError> {
    B256::try_from(bytes).map_err(|_| ChainError::Decode("expected 32 bytes".into()).into())
}

fn timestamp(secs: u64) -> Option<OffsetDateTime> {
    i64::try_from(secs)
        .ok()
        .and_then(|s| OffsetDateTime::from_unix_timestamp(s).ok())
}

impl Engine {
    /// 写入链配置并读取索引进度。
    pub async fn open(pool: PgPool, cfg: &ChainConfig) -> Result<Self, IndexerError> {
        let chain_id = db_i64(cfg.chain_id)?;
        db::upsert_chain(
            &pool,
            db::ChainRow {
                chain_id,
                name: &cfg.name,
                escrow: cfg.contracts.escrow.as_slice(),
                config: cfg.contracts.config.as_slice(),
                token: cfg.token.address.as_slice(),
                token_symbol: &cfg.token.symbol,
                token_decimals: cfg.token.decimals.into(),
                start_block: db_i64(cfg.contracts.start_block)?,
                safe_depth: db_i32(cfg.confirmation.fallback_safe_depth)?,
                finalized_depth: db_i32(cfg.confirmation.fallback_finalized_depth)?,
            },
        )
        .await?;
        let mut engine = Self {
            pool,
            chain_id,
            start_block: cfg.contracts.start_block,
            max_range: cfg.indexer.max_block_range,
            policy: cfg.confirmation.clone(),
            progress: Progress::default(),
        };
        engine.reload().await?;
        Ok(engine)
    }

    async fn reload(&mut self) -> Result<(), IndexerError> {
        let cursor = db::cursor(&self.pool, self.chain_id).await?;
        let checkpoints = db::checkpoints(&self.pool, self.chain_id).await?;
        self.progress = Progress {
            cursor: match &cursor {
                Some(c) => Some(BlockRef {
                    number: db_u64(c.block_number)?,
                    hash: b256(&c.block_hash)?,
                }),
                None => None,
            },
            checkpoints: checkpoints
                .iter()
                .map(|(n, h)| {
                    Ok(BlockRef {
                        number: db_u64(*n)?,
                        hash: b256(h)?,
                    })
                })
                .collect::<Result<_, IndexerError>>()?,
            finalized: cursor.map_or(Ok(0), |c| db_u64(c.finalized_block))?,
        };
        Ok(())
    }

    pub fn progress(&self) -> &Progress {
        &self.progress
    }

    /// 一轮校对：计划 + 执行。返回计划（调用方据 `behind()` 决定是否立即再跑）。
    pub async fn reconcile<S: ChainSource>(
        &mut self,
        src: &S,
    ) -> Result<(Plan, Applied), IndexerError> {
        let plan = reorg::plan(src, &self.progress, self.start_block, self.max_range).await?;
        let applied = self.apply(src, &plan).await;
        // 无论成败都从库里重读进度，内存状态与库保持一致。
        self.reload().await?;
        Ok((plan, applied?))
    }

    async fn apply<S: ChainSource>(&self, src: &S, plan: &Plan) -> Result<Applied, IndexerError> {
        let mut applied = Applied::default();
        // 回滚后分叉点之后的记录都会被删掉，范围内不再有「已存在」的记录。
        let fresh: Vec<&RawLog>;
        let stale: Vec<LogKey>;
        match (&plan.scan, plan.rollback_after) {
            (Some(scan), None) => {
                let stored = db::unfinalized_in_range(
                    &self.pool,
                    self.chain_id,
                    db_i64(scan.from)?,
                    db_i64(scan.to.number)?,
                )
                .await?
                .into_iter()
                .map(|s| {
                    Ok(Stored {
                        key: LogKey {
                            tx_hash: b256(&s.tx_hash)?,
                            log_index: db_u64(s.log_index.into())?,
                        },
                        block_hash: b256(&s.block_hash)?,
                    })
                })
                .collect::<Result<Vec<_>, IndexerError>>()?;
                let d = reorg::diff(&stored, &scan.logs);
                fresh = d.fresh;
                stale = d.stale;
            }
            (Some(scan), Some(_)) => {
                fresh = scan.logs.iter().collect();
                stale = Vec::new();
            }
            (None, _) => {
                fresh = Vec::new();
                stale = Vec::new();
            }
        }
        // RPC 调用放在事务之外。
        let prepared = prepare(src, &fresh).await?;

        let mut tx = self.pool.begin().await?;
        let mut touched: BTreeMap<OrderRef, Cause> = BTreeMap::new();
        if let Some(fork) = plan.rollback_after {
            let refs = db::rollback_after(&mut *tx, self.chain_id, db_i64(fork)?).await?;
            tracing::warn!(
                chain_id = self.chain_id,
                fork,
                orders = refs.len(),
                "chain reorg: rolled back"
            );
            mark(&mut touched, refs, Cause::Reorg);
        }
        if !stale.is_empty() {
            let hashes: Vec<Vec<u8>> = stale.iter().map(|k| k.tx_hash.to_vec()).collect();
            let idx = stale
                .iter()
                .map(|k| db_i32(k.log_index))
                .collect::<Result<Vec<_>, _>>()?;
            let refs = db::delete_events(&mut *tx, self.chain_id, &hashes, &idx).await?;
            tracing::warn!(
                chain_id = self.chain_id,
                logs = stale.len(),
                "removed logs no longer on the main chain"
            );
            applied.removed = stale.len();
            mark(&mut touched, refs, Cause::Reorg);
        }
        for p in &prepared {
            if self.insert(&mut tx, p, true).await? {
                applied.inserted += 1;
                if let Some(r) = p.order_ref() {
                    mark(&mut touched, [r], Cause::Event);
                }
            }
        }
        if let Some(c) = plan.cursor {
            db::set_cursor(
                &mut *tx,
                self.chain_id,
                db_i64(c.number)?,
                c.hash.as_slice(),
            )
            .await?;
        }
        applied.orders_changed = self.recompute(&mut tx, &touched).await?;
        tx.commit().await?;
        Ok(applied)
    }

    /// WebSocket 推送的日志：游标之后的立即写入（状态标为 latest），游标及之前的已由校对覆盖，忽略。
    /// 返回 `true` 表示需要立即校对（节点推送了回滚通知）。
    pub async fn ingest_live<S: ChainSource>(
        &self,
        src: &S,
        log: &RawLog,
    ) -> Result<bool, IndexerError> {
        if log.removed {
            return Ok(true);
        }
        if self
            .progress
            .cursor
            .is_some_and(|c| log.block.number <= c.number)
            || log.block.number < self.start_block
        {
            return Ok(false);
        }
        let prepared = prepare(src, &[log]).await?;
        let mut tx = self.pool.begin().await?;
        let mut touched = BTreeMap::new();
        for p in &prepared {
            if self.insert(&mut tx, p, false).await?
                && let Some(r) = p.order_ref()
            {
                mark(&mut touched, [r], Cause::Event);
            }
        }
        self.recompute(&mut tx, &touched).await?;
        tx.commit().await?;
        Ok(false)
    }

    /// 按 RPC 的 safe / finalized 标记升级确认级别（不超过本轮已核对的游标）。
    pub async fn promote<S: ChainSource>(
        &mut self,
        src: &S,
        head: u64,
    ) -> Result<(u64, u64), IndexerError> {
        let Some(cursor) = self.progress.cursor else {
            return Ok((0, 0));
        };
        let (safe, finalized) = reorg::confirmations(src, &self.policy, head, cursor.number).await;
        // 最终确认只前进不后退（节点切换时两家的 finalized 可能略有差异）。
        let finalized = finalized.max(self.progress.finalized);
        let safe = safe.max(finalized);
        let mut tx = self.pool.begin().await?;
        let mut touched = BTreeMap::new();
        let fin = db::promote(
            &mut *tx,
            self.chain_id,
            db_i64(finalized)?,
            Level::Finalized.as_str(),
        )
        .await?;
        mark(&mut touched, fin, Cause::Confirmation);
        let sf = db::promote(&mut *tx, self.chain_id, db_i64(safe)?, Level::Safe.as_str()).await?;
        mark(&mut touched, sf, Cause::Confirmation);
        db::set_confirmations(&mut *tx, self.chain_id, db_i64(safe)?, db_i64(finalized)?).await?;
        self.recompute(&mut tx, &touched).await?;
        tx.commit().await?;
        self.reload().await?;
        Ok((safe, finalized))
    }

    async fn insert(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        p: &Prepared<'_>,
        replace: bool,
    ) -> Result<bool, IndexerError> {
        let log = p.log;
        let topics: Vec<Vec<u8>> = log.topics.iter().map(|t| t.to_vec()).collect();
        let mut payload = json!({ "args": p.decoded.args });
        if let Some(s) = &p.snapshot {
            payload["snapshot"] =
                serde_json::to_value(s).map_err(|e| ChainError::Decode(e.to_string()))?;
        }
        Ok(db::insert_event(
            &mut **tx,
            db::NewEvent {
                chain_id: self.chain_id,
                contract: log.contract.as_slice(),
                block_number: db_i64(log.block.number)?,
                block_hash: log.block.hash.as_slice(),
                block_time: log.block_timestamp.and_then(timestamp),
                tx_hash: log.key.tx_hash.as_slice(),
                log_index: db_i32(log.key.log_index)?,
                name: &p.decoded.name,
                order_id: p.decoded.order_id.map(db_i64).transpose()?,
                topics: &topics,
                data: &log.data,
                payload: &payload,
            },
            replace,
        )
        .await?)
    }

    /// 重新折叠受影响的订单；行有变化的写一条发件箱消息。返回变化的订单数。
    async fn recompute(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        touched: &BTreeMap<OrderRef, Cause>,
    ) -> Result<usize, IndexerError> {
        let mut changed = 0;
        for ((contract, order_id), cause) in touched {
            let rows = db::order_events(&mut **tx, self.chain_id, contract, *order_id).await?;
            let logs = rows
                .into_iter()
                .map(|r| {
                    Ok(OrderLog {
                        block_number: db_u64(r.block_number)?,
                        log_index: db_u64(r.log_index.into())?,
                        level: Level::parse(&r.level)?,
                        topics: r.topics.iter().map(|t| b256(t)).collect::<Result<_, _>>()?,
                        data: r.data.into(),
                        snapshot: r
                            .snapshot
                            .map(serde_json::from_value::<OrderSnapshot>)
                            .transpose()
                            .map_err(|e| ChainError::Decode(format!("stored snapshot: {e}")))?,
                    })
                })
                .collect::<Result<Vec<_>, IndexerError>>()?;
            let view = order::fold(db_u64(*order_id)?, &logs)?;
            let result = match &view {
                Some(v) => db::upsert_order(&mut **tx, &self.row(contract, v)?).await?,
                None => db::clear_order(&mut **tx, self.chain_id, contract, *order_id).await?,
            };
            let Some(row) = result else { continue };
            changed += 1;
            let contract = Address::try_from(contract.as_slice())
                .map_err(|_| ChainError::Decode("expected 20-byte address".into()))?;
            let payload = json!({
                "id": row.id,
                "chain_id": self.chain_id,
                "contract": contract.to_checksum(None),
                "chain_order_id": order_id,
                "buyer_id": row.buyer_id,
                "seller_id": row.seller_id,
                "state": view.as_ref().map_or("none", |v| v.state.as_str()),
                "level": view.as_ref().map_or(Level::Latest, |v| v.level).as_str(),
                "block_number": view.as_ref().map(|v| v.updated_block),
                "cause": cause.as_str(),
                "correction": *cause == Cause::Reorg,
            });
            outbox::enqueue(&mut **tx, ORDER_UPDATED, &payload, outbox::PRIORITY_NORMAL).await?;
        }
        Ok(changed)
    }

    fn row<'a>(
        &self,
        contract: &'a [u8],
        v: &'a OrderView,
    ) -> Result<db::OrderUpsert<'a>, IndexerError> {
        let dec = |u: &U256| u.to_string();
        let bps = |u: &U256| {
            i32::try_from(*u).map_err(|_| ChainError::Decode(format!("bps {u} out of range")))
        };
        let s = v.snapshot.as_ref();
        Ok(db::OrderUpsert {
            chain_id: self.chain_id,
            contract,
            chain_order_id: db_i64(v.order_id)?,
            buyer: v.buyer.to_vec(),
            seller: v.seller.to_vec(),
            token: v.token.to_vec(),
            amount: dec(&v.amount),
            buyer_fee: s.map(|s| s.buyer_fee.to_string()),
            seller_fee: s.map(|s| s.seller_fee.to_string()),
            buyer_gas: s.map(|s| s.buyer_gas.to_string()),
            seller_gas: s.map(|s| s.seller_gas.to_string()),
            dispute_fee: s.map(|s| s.dispute_fee.to_string()),
            bond: s.map(|s| s.bond.to_string()),
            instant: v.instant,
            offer_hash: v.offer_hash.to_vec(),
            listing_hash: v.listing_hash.to_vec(),
            auction_id: v.auction_id.as_ref().map(dec),
            state: v.state.code(),
            ship_deadline: v.ship_deadline.and_then(timestamp),
            confirm_deadline: v.confirm_deadline.and_then(timestamp),
            proposer: v.proposal.as_ref().map(|p| p.proposer.to_vec()),
            proposal_to_buyer: v.proposal.as_ref().map(|p| dec(&p.to_buyer)),
            proposal_expiry: v.proposal.as_ref().and_then(|p| timestamp(p.expiry)),
            disputed_by: v.dispute.as_ref().map(|d| d.by.to_vec()),
            dispute_from_bond: v.dispute.as_ref().map(|d| d.from_bond),
            ruling_to_buyer_bps: v.ruling.as_ref().map(|r| bps(&r.0)).transpose()?,
            ruling_bond_to_buyer_bps: v.ruling.as_ref().map(|r| bps(&r.1)).transpose()?,
            settled_to_buyer: v.settlement.as_ref().map(|x| dec(&x.to_buyer)),
            settled_to_seller: v.settlement.as_ref().map(|x| dec(&x.to_seller)),
            settled_to_fee: v.settlement.as_ref().map(|x| dec(&x.to_fee_recipient)),
            snapshot: s
                .map(serde_json::to_value)
                .transpose()
                .map_err(|e| ChainError::Decode(e.to_string()))?,
            level: v.level.as_str(),
            created_block: db_i64(v.created_block)?,
            updated_block: db_i64(v.updated_block)?,
        })
    }
}

fn mark(
    touched: &mut BTreeMap<OrderRef, Cause>,
    refs: impl IntoIterator<Item = OrderRef>,
    cause: Cause,
) {
    for r in refs {
        let c = touched.entry(r).or_insert(cause);
        *c = (*c).max(cause);
    }
}

/// 解码好、带快照的待写入日志。
struct Prepared<'a> {
    log: &'a RawLog,
    decoded: event::Decoded,
    snapshot: Option<OrderSnapshot>,
}

impl Prepared<'_> {
    fn order_ref(&self) -> Option<OrderRef> {
        let id = i64::try_from(self.decoded.order_id?).ok()?;
        Some((self.log.contract.to_vec(), id))
    }
}

/// 解码日志；下单事件在其所在区块读取 `getOrder` 快照（每单一次 eth_call）。
/// 无法解码的日志（ABI 之外的事件）照样入库，事件名记为 `unknown`，便于排查。
async fn prepare<'a, S: ChainSource>(
    src: &S,
    logs: &[&'a RawLog],
) -> Result<Vec<Prepared<'a>>, IndexerError> {
    let mut out = Vec::with_capacity(logs.len());
    for log in logs {
        let decoded = event::decode(log).unwrap_or_else(|e| {
            tracing::warn!(tx = %log.key.tx_hash, error = %e, "undecodable log stored as unknown");
            event::Decoded {
                name: "unknown".into(),
                order_id: None,
                args: Value::Null,
            }
        });
        let snapshot = match (decoded.name.as_str(), decoded.order_id) {
            ("OrderCreated", Some(id)) => {
                src.order_snapshot(log.contract, id, log.block.hash).await?
            }
            _ => None,
        };
        out.push(Prepared {
            log,
            decoded,
            snapshot,
        });
    }
    Ok(out)
}
