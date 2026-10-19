//! 区块回滚判定（plan 第 8 节「区块回滚」），与数据库无关的纯逻辑。
//!
//! 每轮校对：
//! 1. 取最新块；核对上一轮游标所在区块的哈希。不符说明发生了回滚：在保存的检查点里二分查找
//!    仍在主链上的最新一个，作为分叉点，此后未最终确认的记录全部撤销；
//! 2. 从游标（或分叉点）之后按区块范围批量 `eth_getLogs`，与库里同一范围的记录对比（[`diff`]）：
//!    WebSocket 先写入但已不在主链上的删除，缺的补上；
//! 3. 游标前移到本轮范围的末块，并保存为新的检查点。
//!
//! 每轮固定约 3 次调用（最新块、游标块、getLogs），与出块速度无关，不逐块轮询。

use std::collections::{HashMap, HashSet};
use std::future::Future;

use alloy::primitives::{Address, B256};

use crate::ChainError;
use crate::config::ConfirmationPolicy;
use crate::event::{BlockRef, LogKey, RawLog};
use crate::snapshot::OrderSnapshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockTag {
    Latest,
    Safe,
    Finalized,
    Number(u64),
}

/// 链数据来源：生产实现是带主备切换的 RPC，测试用内存假链。
pub trait ChainSource: Send + Sync {
    /// 区块号与哈希；区块（还）不存在时返回 `None`。
    fn block(
        &self,
        tag: BlockTag,
    ) -> impl Future<Output = Result<Option<BlockRef>, ChainError>> + Send;

    /// `[from, to]`（含两端）内被索引合约的全部日志。
    fn logs(
        &self,
        from: u64,
        to: u64,
    ) -> impl Future<Output = Result<Vec<RawLog>, ChainError>> + Send;

    /// 在指定区块（按哈希，EIP-1898）读取订单快照。
    fn order_snapshot(
        &self,
        contract: Address,
        order_id: u64,
        at: B256,
    ) -> impl Future<Output = Result<Option<OrderSnapshot>, ChainError>> + Send;
}

/// 库里记录的索引进度。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Progress {
    /// 已完整校对到的区块；`None` = 首次启动。
    pub cursor: Option<BlockRef>,
    /// 近期的游标历史（任意顺序），用来定位分叉点。
    pub checkpoints: Vec<BlockRef>,
    /// 已最终确认的区块号：它及之前的记录不会再被回滚。
    pub finalized: u64,
}

/// 一轮校对要做的事。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub head: BlockRef,
    /// 发生回滚时的分叉点：撤销此区块之后的全部未最终确认记录与检查点。
    pub rollback_after: Option<u64>,
    pub scan: Option<Scan>,
    /// 本轮之后的游标；`None` = 不变。
    pub cursor: Option<BlockRef>,
}

/// 本轮补拉的范围与主链日志（按区块号、日志序号排序）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scan {
    pub from: u64,
    pub to: BlockRef,
    pub logs: Vec<RawLog>,
}

impl Plan {
    /// 范围没到最新块：还在追赶，调用方应立即再跑一轮而不是等下一个周期。
    pub fn behind(&self) -> bool {
        self.scan
            .as_ref()
            .is_some_and(|s| s.to.number < self.head.number)
    }
}

/// 计算一轮校对。只读链，不改任何状态；调用方在一个数据库事务里执行结果。
pub async fn plan<S: ChainSource>(
    src: &S,
    progress: &Progress,
    start_block: u64,
    max_range: u64,
) -> Result<Plan, ChainError> {
    let head = src
        .block(BlockTag::Latest)
        .await?
        .ok_or_else(|| ChainError::Inconsistent("no latest block".into()))?;
    let (base, rollback_after) = match progress.cursor {
        None => (None, None),
        Some(cursor) => {
            if src.block(BlockTag::Number(cursor.number)).await? == Some(cursor) {
                (Some(cursor.number), None)
            } else {
                let fork = fork_point(src, &progress.checkpoints, progress.finalized).await?;
                (Some(fork), Some(fork))
            }
        }
    };
    let from = base.map_or(start_block, |b| (b + 1).max(start_block));
    if from > head.number {
        // 回滚后主链恰好只到分叉点：游标退回分叉点。
        let cursor = match rollback_after {
            Some(fork) => src.block(BlockTag::Number(fork)).await?,
            None => None,
        };
        return Ok(Plan {
            head,
            rollback_after,
            scan: None,
            cursor,
        });
    }
    let to_number = head.number.min(from.saturating_add(max_range.max(1) - 1));
    let to = if to_number == head.number {
        head
    } else {
        src.block(BlockTag::Number(to_number))
            .await?
            .ok_or_else(|| ChainError::Inconsistent(format!("block {to_number} missing")))?
    };
    let logs = checked_logs(src.logs(from, to.number).await?, from, to)?;
    Ok(Plan {
        head,
        rollback_after,
        scan: Some(Scan { from, to, logs }),
        cursor: Some(to),
    })
}

/// 检查日志在范围内、同一区块号只有一个哈希、末块哈希与本轮读到的末块一致；
/// 不一致说明节点在两次调用之间发生了回滚，本轮放弃，下一轮重试。
fn checked_logs(mut logs: Vec<RawLog>, from: u64, to: BlockRef) -> Result<Vec<RawLog>, ChainError> {
    let mut hashes: HashMap<u64, B256> = HashMap::from([(to.number, to.hash)]);
    for log in &logs {
        let n = log.block.number;
        if n < from || n > to.number {
            return Err(ChainError::Inconsistent(format!(
                "log at block {n} outside [{from}, {}]",
                to.number
            )));
        }
        if *hashes.entry(n).or_insert(log.block.hash) != log.block.hash {
            return Err(ChainError::Inconsistent(format!(
                "two hashes for block {n}"
            )));
        }
    }
    logs.retain(|l| !l.removed);
    logs.sort_by_key(|l| (l.block.number, l.key.log_index));
    Ok(logs)
}

/// 二分查找仍在主链上的最新检查点（检查点越新越可能被回滚，是否在主链上关于区块号单调）。
/// 一个都不在时退回最终确认的区块号：那之前的数据不会变。
pub async fn fork_point<S: ChainSource>(
    src: &S,
    checkpoints: &[BlockRef],
    finalized: u64,
) -> Result<u64, ChainError> {
    let mut cps: Vec<BlockRef> = checkpoints
        .iter()
        .copied()
        .filter(|c| c.number > finalized)
        .collect();
    cps.sort_by_key(|c| c.number);
    // 不变量：cps[..lo] 在主链上，cps[hi..] 不在。
    let (mut lo, mut hi) = (0, cps.len());
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let cp = cps[mid];
        if src.block(BlockTag::Number(cp.number)).await? == Some(cp) {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    Ok(lo.checked_sub(1).map_or(finalized, |i| cps[i].number))
}

/// 库里已有的一条记录。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stored {
    pub key: LogKey,
    pub block_hash: B256,
}

/// 同一区块范围内库与主链的差异。先删 `stale` 再插 `fresh`（同一日志换了区块时两边都有它）。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Diff<'a> {
    pub stale: Vec<LogKey>,
    pub fresh: Vec<&'a RawLog>,
}

pub fn diff<'a>(stored: &[Stored], canonical: &'a [RawLog]) -> Diff<'a> {
    let canon: HashSet<(LogKey, B256)> = canonical.iter().map(|l| (l.key, l.block.hash)).collect();
    let have: HashSet<(LogKey, B256)> = stored.iter().map(|s| (s.key, s.block_hash)).collect();
    Diff {
        stale: stored
            .iter()
            .filter(|s| !canon.contains(&(s.key, s.block_hash)))
            .map(|s| s.key)
            .collect(),
        fresh: canonical
            .iter()
            .filter(|l| !have.contains(&(l.key, l.block.hash)))
            .collect(),
    }
}

/// 本轮的 safe / finalized 区块号，不超过 `verified`（本轮已核对在主链上的游标）。
/// 节点不支持区块标记（或查询失败）时退回固定深度。
pub async fn confirmations<S: ChainSource>(
    src: &S,
    policy: &ConfirmationPolicy,
    head: u64,
    verified: u64,
) -> (u64, u64) {
    let tag = |t| async move {
        if !policy.use_block_tags {
            return None;
        }
        match src.block(t).await {
            Ok(b) => b.map(|b| b.number),
            Err(e) => {
                tracing::warn!(error = %e, tag = ?t, "block tag query failed, using fixed depth");
                None
            }
        }
    };
    let finalized = tag(BlockTag::Finalized)
        .await
        .unwrap_or_else(|| head.saturating_sub(policy.fallback_finalized_depth));
    let safe = tag(BlockTag::Safe)
        .await
        .unwrap_or_else(|| head.saturating_sub(policy.fallback_safe_depth))
        .max(finalized);
    (safe.min(verified), finalized.min(verified))
}

#[cfg(test)]
pub mod tests {
    #![allow(clippy::unwrap_used)]

    use std::collections::BTreeMap;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use alloy::primitives::{Bytes, U256};

    use super::*;

    type LogsByBlock = BTreeMap<u64, Vec<u64>>;

    /// 内存假链：`blocks[n]` 是第 n 块的哈希，`logs[n]` 是第 n 块里的日志（交易号）。
    #[derive(Default)]
    pub struct FakeChain {
        inner: Mutex<(Vec<B256>, LogsByBlock)>,
        salt: AtomicUsize,
        pub calls: AtomicUsize,
    }

    impl FakeChain {
        pub fn new(len: u64) -> Self {
            let c = Self::default();
            c.extend(len);
            c
        }

        fn fresh_hash(&self, n: u64) -> B256 {
            let salt = self.salt.fetch_add(1, Ordering::Relaxed) as u64;
            B256::from(U256::from(n) << 64 | U256::from(salt))
        }

        pub fn extend(&self, by: u64) {
            for _ in 0..by {
                let n = self.inner.lock().unwrap().0.len() as u64;
                let h = self.fresh_hash(n);
                self.inner.lock().unwrap().0.push(h);
            }
        }

        /// 在第 `n` 块放一条日志，交易号 `tx`。
        pub fn emit(&self, n: u64, tx: u64) {
            self.inner.lock().unwrap().1.entry(n).or_default().push(tx);
        }

        /// 丢掉 `fork_after` 之后的块与日志，再出 `new_len - fork_after - 1` 个新块。
        pub fn reorg(&self, fork_after: u64, new_len: u64) {
            {
                let mut g = self.inner.lock().unwrap();
                g.0.truncate(usize::try_from(fork_after + 1).unwrap());
                g.1.retain(|n, _| *n <= fork_after);
            }
            self.extend(new_len - fork_after - 1);
        }

        pub fn head(&self) -> u64 {
            self.inner.lock().unwrap().0.len() as u64 - 1
        }

        pub fn hash(&self, n: u64) -> B256 {
            self.inner.lock().unwrap().0[usize::try_from(n).unwrap()]
        }

        pub fn canonical(&self) -> Vec<(LogKey, B256)> {
            let g = self.inner.lock().unwrap();
            let mut v: Vec<_> =
                g.1.iter()
                    .flat_map(|(n, txs)| {
                        txs.iter()
                            .enumerate()
                            .map(|(i, tx)| (key(*tx, i as u64), g.0[*n as usize]))
                    })
                    .collect();
            v.sort();
            v
        }

        pub fn log(&self, n: u64, i: u64, tx: u64) -> RawLog {
            RawLog {
                contract: Address::ZERO,
                block: BlockRef {
                    number: n,
                    hash: self.hash(n),
                },
                key: key(tx, i),
                topics: vec![],
                data: Bytes::new(),
                block_timestamp: None,
                removed: false,
            }
        }
    }

    pub fn key(tx: u64, log_index: u64) -> LogKey {
        LogKey {
            tx_hash: B256::from(U256::from(tx)),
            log_index,
        }
    }

    impl ChainSource for FakeChain {
        async fn block(&self, tag: BlockTag) -> Result<Option<BlockRef>, ChainError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let head = self.head();
            let n = match tag {
                BlockTag::Latest => head,
                BlockTag::Safe => head.saturating_sub(2),
                BlockTag::Finalized => head.saturating_sub(4),
                BlockTag::Number(n) if n > head => return Ok(None),
                BlockTag::Number(n) => n,
            };
            Ok(Some(BlockRef {
                number: n,
                hash: self.hash(n),
            }))
        }

        async fn logs(&self, from: u64, to: u64) -> Result<Vec<RawLog>, ChainError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let txs: Vec<(u64, Vec<u64>)> = self
                .inner
                .lock()
                .unwrap()
                .1
                .range(from..=to)
                .map(|(n, t)| (*n, t.clone()))
                .collect();
            Ok(txs
                .into_iter()
                .flat_map(|(n, t)| {
                    t.into_iter()
                        .enumerate()
                        .map(move |(i, tx)| (n, i as u64, tx))
                })
                .map(|(n, i, tx)| self.log(n, i, tx))
                .collect())
        }

        async fn order_snapshot(
            &self,
            _: Address,
            _: u64,
            _: B256,
        ) -> Result<Option<OrderSnapshot>, ChainError> {
            Ok(None)
        }
    }

    /// 内存「数据库」：按与 forum-indexer 相同的规则执行计划。
    #[derive(Default)]
    struct Store {
        events: BTreeMap<LogKey, (u64, B256, bool)>, // 区块号、哈希、是否最终确认
        progress: Progress,
        rolled_back: Vec<u64>,
    }

    impl Store {
        fn apply(&mut self, p: &Plan) {
            if let Some(fork) = p.rollback_after {
                self.rolled_back.push(fork);
                self.events.retain(|_, (n, _, fin)| *n <= fork || *fin);
                self.progress.checkpoints.retain(|c| c.number <= fork);
            }
            if let Some(scan) = &p.scan {
                let stored: Vec<Stored> = self
                    .events
                    .iter()
                    .filter(|(_, (n, _, fin))| *n >= scan.from && *n <= scan.to.number && !fin)
                    .map(|(k, (_, h, _))| Stored {
                        key: *k,
                        block_hash: *h,
                    })
                    .collect();
                let d = diff(&stored, &scan.logs);
                for k in d.stale {
                    self.events.remove(&k);
                }
                for l in d.fresh {
                    self.events
                        .insert(l.key, (l.block.number, l.block.hash, false));
                }
                self.progress.checkpoints.push(scan.to);
            }
            if let Some(c) = p.cursor {
                self.progress.cursor = Some(c);
            }
        }

        /// WebSocket 即时写入（不经过校对）。
        fn ws_insert(&mut self, l: &RawLog) {
            self.events
                .insert(l.key, (l.block.number, l.block.hash, false));
        }

        fn finalize(&mut self, upto: u64) {
            for (n, _, fin) in self.events.values_mut() {
                if *n <= upto {
                    *fin = true;
                }
            }
            self.progress.finalized = upto;
        }

        fn snapshot(&self) -> Vec<(LogKey, B256)> {
            self.events.iter().map(|(k, (_, h, _))| (*k, *h)).collect()
        }

        async fn sync(&mut self, chain: &FakeChain, range: u64) {
            for _ in 0..1000 {
                let p = plan(chain, &self.progress, 0, range).await.unwrap();
                self.apply(&p);
                if !p.behind() {
                    return;
                }
            }
            panic!("did not converge");
        }
    }

    #[tokio::test]
    async fn catch_up_in_chunks_matches_chain() {
        let chain = FakeChain::new(50);
        for (n, tx) in [(3, 1), (3, 2), (17, 3), (49, 4)] {
            chain.emit(n, tx);
        }
        let mut s = Store::default();
        s.sync(&chain, 10).await;
        assert_eq!(s.snapshot(), chain.canonical());
        assert_eq!(s.progress.cursor.unwrap().number, 49);
        assert_eq!(
            s.progress.checkpoints.len(),
            5,
            "one checkpoint per 10-block chunk"
        );
        assert!(s.rolled_back.is_empty());
    }

    #[tokio::test]
    async fn steady_state_costs_three_calls_per_tick() {
        let chain = FakeChain::new(10);
        let mut s = Store::default();
        s.sync(&chain, 100).await;
        chain.extend(20);
        chain.calls.store(0, Ordering::Relaxed);
        s.sync(&chain, 100).await;
        assert_eq!(chain.calls.load(Ordering::Relaxed), 3);
    }

    #[tokio::test]
    async fn reorg_rolls_back_and_replays() {
        let chain = FakeChain::new(20);
        chain.emit(5, 1);
        chain.emit(15, 2);
        chain.emit(18, 3);
        let mut s = Store::default();
        s.sync(&chain, 4).await; // 检查点 3,7,11,15,19
        assert_eq!(s.snapshot(), chain.canonical());

        // 第 13 块之后分叉：交易 2 重新打包进第 16 块，交易 3 消失，新增交易 4。
        chain.reorg(13, 22);
        chain.emit(16, 2);
        chain.emit(21, 4);
        s.sync(&chain, 4).await;
        assert_eq!(
            s.rolled_back,
            vec![11],
            "newest checkpoint still on the main chain"
        );
        assert_eq!(s.snapshot(), chain.canonical());
        assert_eq!(s.events[&key(2, 0)].0, 16, "replayed at its new block");
        assert!(!s.events.contains_key(&key(3, 0)));
    }

    #[tokio::test]
    async fn reorg_to_shorter_chain() {
        let chain = FakeChain::new(30);
        chain.emit(25, 1);
        let mut s = Store::default();
        s.sync(&chain, 100).await;
        s.progress.checkpoints.push(BlockRef {
            number: 20,
            hash: chain.hash(20),
        });
        chain.reorg(22, 24); // 新链只到第 23 块，游标第 29 块已不存在
        s.sync(&chain, 100).await;
        assert_eq!(s.rolled_back, vec![20]);
        assert_eq!(s.progress.cursor.unwrap().number, 23);
        assert_eq!(s.snapshot(), chain.canonical());
        assert!(s.snapshot().is_empty());
    }

    #[tokio::test]
    async fn stale_websocket_log_beyond_cursor_is_removed() {
        let chain = FakeChain::new(10);
        let mut s = Store::default();
        s.sync(&chain, 100).await;
        // WebSocket 推送了第 11 块的日志，随后这一块被替换且日志没进新块。
        chain.extend(2);
        chain.emit(11, 7);
        s.ws_insert(&chain.log(11, 0, 7));
        chain.reorg(10, 13);
        s.sync(&chain, 100).await;
        assert!(s.rolled_back.is_empty(), "cursor (block 9) unaffected");
        assert!(s.snapshot().is_empty());
    }

    #[tokio::test]
    async fn websocket_duplicate_is_not_inserted_twice() {
        let chain = FakeChain::new(10);
        let mut s = Store::default();
        s.sync(&chain, 100).await;
        chain.extend(1);
        chain.emit(10, 1);
        s.ws_insert(&chain.log(10, 0, 1));
        let p = plan(&chain, &s.progress, 0, 100).await.unwrap();
        let stored = [Stored {
            key: key(1, 0),
            block_hash: chain.hash(10),
        }];
        assert_eq!(diff(&stored, &p.scan.unwrap().logs), Diff::default());
    }

    #[tokio::test]
    async fn finalized_records_survive_and_bound_the_fork_search() {
        let chain = FakeChain::new(20);
        chain.emit(3, 1);
        chain.emit(12, 2);
        let mut s = Store::default();
        s.sync(&chain, 5).await; // 检查点 4,9,14,19
        s.finalize(9);
        // 分叉早于所有未最终确认的检查点：退回最终确认的第 9 块。
        chain.reorg(10, 25);
        chain.emit(11, 3);
        s.sync(&chain, 5).await;
        assert_eq!(s.rolled_back, vec![9]);
        assert!(s.events.contains_key(&key(1, 0)), "finalized row kept");
        assert_eq!(s.snapshot(), chain.canonical());
    }

    #[tokio::test]
    async fn fork_search_is_logarithmic() {
        let chain = FakeChain::new(1001);
        let cps: Vec<BlockRef> = (1..=1000)
            .map(|n| BlockRef {
                number: n,
                hash: chain.hash(n),
            })
            .collect();
        chain.reorg(700, 1001);
        chain.calls.store(0, Ordering::Relaxed);
        assert_eq!(fork_point(&chain, &cps, 0).await.unwrap(), 700);
        assert!(chain.calls.load(Ordering::Relaxed) <= 11);
    }

    #[test]
    fn inconsistent_logs_are_rejected() {
        let chain = FakeChain::new(11);
        let to = BlockRef {
            number: 9,
            hash: chain.hash(9),
        };
        let mut wrong = chain.log(9, 0, 1);
        wrong.block.hash = B256::repeat_byte(9);
        assert!(checked_logs(vec![wrong], 0, to).is_err());
        assert!(checked_logs(vec![chain.log(10, 0, 1)], 0, to).is_err());
        let (mut a, mut b) = (chain.log(5, 0, 1), chain.log(5, 1, 2));
        a.block.hash = B256::repeat_byte(1);
        b.block.hash = B256::repeat_byte(2);
        assert!(checked_logs(vec![a, b], 0, to).is_err());
    }

    #[tokio::test]
    async fn confirmations_use_tags_and_never_pass_verified() {
        let chain = FakeChain::new(101);
        let tags = ConfirmationPolicy::default();
        assert_eq!(confirmations(&chain, &tags, 100, 100).await, (98, 96));
        assert_eq!(confirmations(&chain, &tags, 100, 97).await, (97, 96));
        let depth = ConfirmationPolicy {
            use_block_tags: false,
            fallback_safe_depth: 10,
            fallback_finalized_depth: 50,
        };
        assert_eq!(confirmations(&chain, &depth, 100, 100).await, (90, 50));
    }
}
