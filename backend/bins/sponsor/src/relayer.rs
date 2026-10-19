//! 私有代发器（plan 第 5 节）：用唯一的热钱包 EOA 直接调用 EntryPoint v0.7 的 `handleOps`。
//!
//! - 微批：收到第一笔后等一个窗口（100–200 ms），把窗口内不同账户的操作合进一个 `handleOps`；
//!   每个账户同时只有一笔待上链（签名时已保证），同一批里不会冲突。
//! - 一次只有一笔交易在路上：nonce 本地递增，出错时重新向节点要 pending nonce。
//! - 提交前 `eth_call` 预演整批：某一笔验证失败（`FailedOp`）就把它剔除再试，不让一笔坏操作拖垮整批；
//!   还没到 `validAfter`（AA32，节点最新块时间略早于本机时钟）的放回队列稍后重试。
//! - 超过 `resend_after` 没上链就同 nonce 提价重发；超过 `max_bumps` 次仍未上链视为不健康，
//!   新操作改走第三方 bundler（见 [`crate::bundler`]），冷却 [`COOLDOWN`] 后再试着自己代发。

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use alloy::consensus::{SignableTransaction, TxEip1559, TxEnvelope};
use alloy::eips::eip2718::Encodable2718;
use alloy::network::TxSignerSync;
use alloy::primitives::{Address, B256, Bytes, TxKind, U256};
use alloy::rpc::types::TransactionReceipt;
use alloy::signers::local::PrivateKeySigner;
use alloy::sol_types::{SolCall, SolInterface};
use forum_chain::aa::{IEntryPoint, PackedUserOperation};
use forum_db::PgPool;
use forum_db::sponsor::{self as db, SponsorshipStatus};
use time::OffsetDateTime;
use tokio::sync::{Mutex, Notify};
use tokio_util::sync::CancellationToken;

use crate::chain::{Chain, NodeError, call_request};
use crate::config::RelayerConfig;
use crate::ledger;

/// 连续几次提交失败算不健康。
const MAX_SEND_FAILURES: u32 = 3;
/// 不健康后多久再试着自己代发。
pub const COOLDOWN: Duration = Duration::from_secs(300);
const RECEIPT_POLL: Duration = Duration::from_millis(250);
/// 卡住的交易最多再等多久（签名 2 分钟后就过期，再等没有意义）。
const STUCK_GIVE_UP: Duration = Duration::from_secs(150);
/// 交易 gas 上限在估算值上多留 20%。
const GAS_LIMIT_MARGIN: (u64, u64) = (6, 5);
/// 提价重发：每次 +25%（节点要求替换交易至少 +10%）。
const BUMP: (u128, u128) = (5, 4);

/// 排队等待上链的一笔操作。
#[derive(Debug, Clone)]
pub struct Queued {
    pub user_op_hash: B256,
    pub op: PackedUserOperation,
    pub valid_until: u64,
}

#[derive(Clone)]
pub struct Relayer {
    inner: Arc<Inner>,
}

struct Inner {
    db: PgPool,
    chain: Arc<Chain>,
    signer: PrivateKeySigner,
    cfg: RelayerConfig,
    queue: Mutex<VecDeque<Queued>>,
    notify: Notify,
    send_failures: AtomicU32,
    /// 在这个时间（Unix 毫秒）之前视为不健康。
    unhealthy_until_ms: AtomicU64,
    balance_ok: AtomicBool,
}

fn now_secs() -> u64 {
    u64::try_from(OffsetDateTime::now_utc().unix_timestamp()).unwrap_or(0)
}

fn now_ms() -> u64 {
    u64::try_from(OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000).unwrap_or(0)
}

/// 预演失败的那一笔：下标与 EntryPoint 给出的原因（AAxx）。
fn failed_op(e: &NodeError) -> Option<(usize, String)> {
    let data = e.data.as_ref()?;
    match IEntryPoint::IEntryPointErrors::abi_decode(data).ok()? {
        IEntryPoint::IEntryPointErrors::FailedOp(f) => {
            Some((usize::try_from(f.opIndex).ok()?, f.reason))
        }
        IEntryPoint::IEntryPointErrors::FailedOpWithRevert(f) => {
            Some((usize::try_from(f.opIndex).ok()?, f.reason))
        }
    }
}

/// 一批里每个账户只取一笔，保持先来先上。其余的留在队列里。
fn take_batch(queue: &mut VecDeque<Queued>, max: usize) -> Vec<Queued> {
    let mut senders = HashSet::new();
    let mut batch = Vec::new();
    let mut rest = VecDeque::with_capacity(queue.len());
    while let Some(q) = queue.pop_front() {
        if batch.len() < max && senders.insert(q.op.sender) {
            batch.push(q);
        } else {
            rest.push_back(q);
        }
    }
    *queue = rest;
    batch
}

enum Sent {
    Mined(Box<TransactionReceipt>),
    /// 节点或热钱包出问题，操作放回队列。
    Retry,
    /// 一直没上链，交给对账任务。
    GaveUp,
}

impl Relayer {
    pub fn new(
        db: PgPool,
        chain: Arc<Chain>,
        signer: PrivateKeySigner,
        cfg: RelayerConfig,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                db,
                chain,
                signer,
                cfg,
                queue: Mutex::new(VecDeque::new()),
                notify: Notify::new(),
                send_failures: AtomicU32::new(0),
                unhealthy_until_ms: AtomicU64::new(0),
                balance_ok: AtomicBool::new(true),
            }),
        }
    }

    /// 热钱包地址（也是 `handleOps` 的 beneficiary）。
    pub fn address(&self) -> Address {
        self.inner.signer.address()
    }

    /// 能否自己代发；否则新操作转交第三方 bundler。
    pub fn healthy(&self) -> bool {
        let i = &self.inner;
        i.balance_ok.load(Ordering::Relaxed)
            && now_ms() >= i.unhealthy_until_ms.load(Ordering::Relaxed)
    }

    fn mark_unhealthy(&self) {
        let until = now_ms() + u64::try_from(COOLDOWN.as_millis()).unwrap_or(u64::MAX);
        self.inner
            .unhealthy_until_ms
            .store(until, Ordering::Relaxed);
    }

    /// 余额巡检的结果（低于告警线视为不健康）。
    pub fn set_balance_ok(&self, ok: bool) {
        self.inner.balance_ok.store(ok, Ordering::Relaxed);
    }

    pub async fn submit(&self, q: Queued) {
        self.inner.queue.lock().await.push_back(q);
        self.inner.notify.notify_one();
    }

    pub async fn run(self, shutdown: CancellationToken) {
        let mut nonce = None;
        loop {
            if self.inner.queue.lock().await.is_empty() {
                tokio::select! {
                    () = shutdown.cancelled() => return,
                    () = self.inner.notify.notified() => {}
                }
            }
            tokio::select! {
                () = shutdown.cancelled() => return,
                () = tokio::time::sleep(self.inner.cfg.batch_window) => {}
            }
            let batch = {
                let mut q = self.inner.queue.lock().await;
                take_batch(&mut q, self.inner.cfg.max_batch)
            };
            if !batch.is_empty() {
                self.process(batch, &mut nonce).await;
            }
        }
    }

    async fn requeue(&self, ops: Vec<Queued>) {
        let mut q = self.inner.queue.lock().await;
        for op in ops.into_iter().rev() {
            q.push_front(op);
        }
    }

    async fn close(&self, q: &Queued, status: SponsorshipStatus) {
        if let Err(e) = db::close(&self.inner.db, q.user_op_hash.as_slice(), status).await {
            tracing::error!(error = %e, "close sponsorship failed");
        }
    }

    fn send_failed(&self) {
        if self.inner.send_failures.fetch_add(1, Ordering::Relaxed) + 1 >= MAX_SEND_FAILURES {
            self.inner.send_failures.store(0, Ordering::Relaxed);
            tracing::error!("repeated handleOps failures; relayer unhealthy");
            self.mark_unhealthy();
        }
    }

    /// 预演整批，剔除验证不通过的操作。返回能上链的那些；节点故障时返回 `None`（已放回队列）。
    async fn preflight(&self, mut ops: Vec<Queued>) -> Option<Vec<Queued>> {
        let ep = self.inner.chain.entry_point;
        while !ops.is_empty() {
            let tx = call_request(self.address(), ep, self.handle_ops(&ops));
            let err = match self.inner.chain.call(&tx).await {
                Ok(Ok(_)) => return Some(ops),
                Ok(Err(e)) => e,
                Err(e) => {
                    tracing::warn!(error = %e, "handleOps preflight: rpc unavailable");
                    self.send_failed();
                    self.requeue(ops).await;
                    return None;
                }
            };
            let Some((i, reason)) = failed_op(&err).filter(|(i, _)| *i < ops.len()) else {
                tracing::warn!(error = %err, ops = ops.len(), "handleOps preflight reverted; dropping batch");
                for q in &ops {
                    self.close(q, SponsorshipStatus::Dropped).await;
                }
                return Some(Vec::new());
            };
            let q = ops.remove(i);
            let live = now_secs() < q.valid_until;
            if reason.starts_with("AA32") && live {
                // 节点最新块的时间还没到 validAfter：稍后再试。
                self.requeue(vec![q]).await;
            } else {
                tracing::warn!(user_op_hash = %q.user_op_hash, %reason, "dropping user operation");
                let status = if live {
                    SponsorshipStatus::Dropped
                } else {
                    SponsorshipStatus::Expired
                };
                self.close(&q, status).await;
            }
        }
        Some(ops)
    }

    fn handle_ops(&self, ops: &[Queued]) -> Bytes {
        IEntryPoint::handleOpsCall {
            ops: ops.iter().map(|q| q.op.clone()).collect(),
            beneficiary: self.address(),
        }
        .abi_encode()
        .into()
    }

    async fn process(&self, batch: Vec<Queued>, nonce: &mut Option<u64>) {
        let now = now_secs();
        let (live, expired): (Vec<_>, Vec<_>) =
            batch.into_iter().partition(|q| q.valid_until > now);
        for q in &expired {
            self.close(q, SponsorshipStatus::Expired).await;
        }
        let Some(ops) = self.preflight(live).await else {
            return;
        };
        if ops.is_empty() {
            return;
        }
        match self.send(&ops, nonce).await {
            Sent::Mined(receipt) => {
                self.inner.send_failures.store(0, Ordering::Relaxed);
                self.inner.unhealthy_until_ms.store(0, Ordering::Relaxed);
                self.settle(&ops, &receipt).await;
            }
            Sent::Retry => {
                self.send_failed();
                self.requeue(ops).await;
            }
            Sent::GaveUp => {}
        }
    }

    async fn settle(&self, ops: &[Queued], receipt: &TransactionReceipt) {
        let found = ledger::outcomes(receipt.inner.logs(), self.inner.chain.entry_point);
        for o in &found {
            if let Err(e) = ledger::record(&self.inner.db, o).await {
                tracing::error!(error = %e, user_op_hash = %o.user_op_hash, "record outcome failed");
            }
        }
        tracing::info!(
            tx = %receipt.transaction_hash,
            ops = ops.len(),
            included = found.len(),
            "handleOps mined"
        );
        if !receipt.status() {
            // 整笔 handleOps 回滚：没有任何操作执行，签名可能还有效但不再重试。
            for q in ops {
                self.close(q, SponsorshipStatus::Dropped).await;
            }
        }
    }

    fn sign(&self, tx: &mut TxEip1559) -> anyhow::Result<Vec<u8>> {
        let sig = self.inner.signer.sign_transaction_sync(tx)?;
        let envelope: TxEnvelope = tx.clone().into_signed(sig).into();
        Ok(envelope.encoded_2718())
    }

    async fn send(&self, ops: &[Queued], nonce: &mut Option<u64>) -> Sent {
        match self.try_send(ops, nonce).await {
            Ok(sent) => sent,
            Err(e) => {
                tracing::warn!(error = %e, "handleOps submission failed");
                *nonce = None;
                Sent::Retry
            }
        }
    }

    async fn try_send(&self, ops: &[Queued], nonce: &mut Option<u64>) -> anyhow::Result<Sent> {
        let chain = &self.inner.chain;
        let input = self.handle_ops(ops);
        let estimate_req = call_request(self.address(), chain.entry_point, input.clone());
        let gas = chain
            .estimate_gas(&estimate_req)
            .await?
            .map_err(|e| anyhow::anyhow!("estimateGas: {e}"))?;
        let gas_price = chain.gas_price().await?;
        let priority = chain.max_priority_fee().await.unwrap_or(0);
        let n = match *nonce {
            Some(n) => n,
            None => chain.pending_nonce(self.address()).await?,
        };
        let mut tx = TxEip1559 {
            chain_id: chain.chain_id,
            nonce: n,
            gas_limit: gas * GAS_LIMIT_MARGIN.0 / GAS_LIMIT_MARGIN.1,
            max_fee_per_gas: gas_price.saturating_mul(2),
            max_priority_fee_per_gas: priority.min(gas_price.saturating_mul(2)),
            to: TxKind::Call(chain.entry_point),
            value: U256::ZERO,
            access_list: Default::default(),
            input,
        };
        let mut hashes: Vec<B256> = Vec::new();
        let started = Instant::now();
        let mut bumps = 0;
        loop {
            let raw = self.sign(&mut tx)?;
            match chain.send_raw(&raw).await? {
                Ok(h) => {
                    tracing::info!(tx = %h, nonce = n, bumps, ops = ops.len(), "handleOps sent");
                    hashes.push(h);
                }
                // 第一次就被拒（nonce 不对、余额不足……）：重新取 nonce 再试。
                Err(e) if hashes.is_empty() => anyhow::bail!("sendRawTransaction: {e}"),
                // 替换交易被拒，多半是原交易刚刚上链：继续等回执。
                Err(e) => tracing::warn!(error = %e, "fee bump rejected"),
            }
            if let Some(r) = self
                .wait_receipt(&hashes, self.inner.cfg.resend_after)
                .await?
            {
                *nonce = Some(n + 1);
                return Ok(Sent::Mined(Box::new(r)));
            }
            if bumps >= self.inner.cfg.max_bumps {
                break;
            }
            bumps += 1;
            tx.max_fee_per_gas = tx.max_fee_per_gas * BUMP.0 / BUMP.1;
            tx.max_priority_fee_per_gas = (tx.max_priority_fee_per_gas * BUMP.0 / BUMP.1).max(1);
        }
        self.mark_unhealthy();
        tracing::error!(
            nonce = n,
            "handleOps stuck after fee bumps; relayer unhealthy"
        );
        let remaining = STUCK_GIVE_UP.saturating_sub(started.elapsed());
        if let Some(r) = self.wait_receipt(&hashes, remaining).await? {
            *nonce = Some(n + 1);
            return Ok(Sent::Mined(Box::new(r)));
        }
        *nonce = None;
        Ok(Sent::GaveUp)
    }

    async fn wait_receipt(
        &self,
        hashes: &[B256],
        timeout: Duration,
    ) -> anyhow::Result<Option<TransactionReceipt>> {
        let deadline = Instant::now() + timeout;
        loop {
            for h in hashes {
                if let Some(r) = self.inner.chain.receipt(*h).await? {
                    return Ok(Some(r));
                }
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            tokio::time::sleep(RECEIPT_POLL).await;
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use alloy::sol_types::SolError;

    use super::*;

    fn queued(sender: u8, n: u8) -> Queued {
        Queued {
            user_op_hash: B256::repeat_byte(n),
            op: PackedUserOperation {
                sender: Address::repeat_byte(sender),
                nonce: U256::from(n),
                initCode: Bytes::new(),
                callData: Bytes::new(),
                accountGasLimits: B256::ZERO,
                preVerificationGas: U256::ZERO,
                gasFees: B256::ZERO,
                paymasterAndData: Bytes::new(),
                signature: Bytes::new(),
            },
            valid_until: 0,
        }
    }

    #[test]
    fn batches_take_one_op_per_sender_in_order() {
        let mut q: VecDeque<_> = [queued(1, 1), queued(1, 2), queued(2, 3), queued(3, 4)].into();
        let b = take_batch(&mut q, 2);
        assert_eq!(
            b.iter().map(|q| q.user_op_hash[0]).collect::<Vec<_>>(),
            [1, 3]
        );
        assert_eq!(
            q.iter().map(|q| q.user_op_hash[0]).collect::<Vec<_>>(),
            [2, 4]
        );
    }

    #[test]
    fn decodes_failed_op_index_and_reason() {
        let data = IEntryPoint::FailedOp {
            opIndex: U256::from(1),
            reason: "AA32 paymaster expired or not due".into(),
        }
        .abi_encode();
        let e = NodeError {
            code: 3,
            message: "execution reverted".into(),
            data: Some(data.into()),
        };
        let (i, reason) = failed_op(&e).unwrap();
        assert_eq!(i, 1);
        assert!(reason.starts_with("AA32"));
        assert!(failed_op(&NodeError { data: None, ..e }).is_none());
    }
}
