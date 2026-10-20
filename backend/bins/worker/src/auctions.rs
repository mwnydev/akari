//! 拍卖结算（plan 第 6 节「成交」「赢家违约」）：结束 → 成交证明 / 流拍；付款期过 → 违约、禁拍、次高者递补。
//!
//! 每轮（[`INTERVAL`]）：
//! 1. 结束时间已到的进行中拍卖：没有出价或最高价未达保留价 → 流拍；否则从最高出价者起挑第一位仍符合资格的人
//!    （没被封号 / 禁拍、有钱包、钱包余额满足卖家的出价资格——结束时再校验一次，plan 第 6 节），不符合的跳过
//!    （`auction_passes`，不算违约），请 forum-sponsor 签第 1 份成交证明，拍卖进入待付款。
//! 2. 付款截止时间已过 [`Config::grace`] 的待付款拍卖：先读链上 `auctionSettled`（indexer 可能还没跟上）；确实没付款
//!    → 记违约、自动禁拍（`forum_db::auctions::record_default`），再按同样的规则给下一位发递补证明（递补期另算），
//!    最多 3 份；没人可递补或已用完 → 流拍。
//!
//! 成交证明由 forum-sponsor 独立核对后签名（本程序不持有任何私钥）；签发失败（被拒或服务不可用）时下一轮重试。
//! 链上成交由 forum-indexer 标记（`forum_db::auctions::sync_order`）。

use std::time::Duration;

use alloy::primitives::{Address, U256};
use anyhow::Context;
use forum_chain::rpc::FailoverRpc;
use forum_core::Usdt;
use forum_core::auction::{Eligibility, MAX_ROUNDS};
use forum_core::chat::Event;
use forum_db::auctions::{self as db, Auction};
use forum_db::{PgPool, sanctions};
use forum_kv::Realtime;
use forum_kv::guest_cache::{self, GuestCache};
use forum_kv::realtime::{Envelope, Route, Topic};
use serde::Deserialize;
use serde_json::json;
use time::OffsetDateTime;

/// 检查间隔：结束与付款截止都按分钟精度处理就够（付款截止由合约按区块时间强制）。
pub const INTERVAL: Duration = Duration::from_secs(15);
/// 每轮每类最多处理几场。
const BATCH: i64 = 50;

/// 结算需要的外部服务：链上读取与 forum-sponsor 的成交证明。生产实现是 [`Live`]，测试用假实现。
#[async_trait::async_trait]
pub trait Settlement: Send + Sync {
    /// 钱包的 USDT 余额（出价资格）。
    async fn usdt_balance(&self, wallet: &[u8]) -> anyhow::Result<Usdt>;
    /// 链上这场拍卖是否已成交。
    async fn settled_on_chain(&self, seller: &[u8], auction_id: i64) -> anyhow::Result<bool>;
    /// 请 forum-sponsor 签第 `round` 份证明给出价 `bid_id`，返回付款截止时间。
    async fn issue_proof(
        &self,
        auction_id: i64,
        round: i16,
        bid_id: i64,
    ) -> anyhow::Result<OffsetDateTime>;
}

#[derive(Debug, Clone, Copy)]
pub struct Config {
    /// 付款截止后再等多久才判违约：给 indexer 留出看到截止前最后一笔付款的时间。
    pub grace: time::Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            grace: time::Duration::minutes(5),
        }
    }
}

impl Config {
    /// `AUCTION_DEFAULT_GRACE_SECS`（默认 300，60–3600）。
    pub fn from_env() -> anyhow::Result<Self> {
        let Some(v) = std::env::var("AUCTION_DEFAULT_GRACE_SECS")
            .ok()
            .filter(|v| !v.is_empty())
        else {
            return Ok(Self::default());
        };
        let secs: i64 = v
            .parse()
            .context("AUCTION_DEFAULT_GRACE_SECS must be a number")?;
        anyhow::ensure!(
            (60..=3600).contains(&secs),
            "AUCTION_DEFAULT_GRACE_SECS must be 60–3600"
        );
        Ok(Self {
            grace: time::Duration::seconds(secs),
        })
    }
}

/// 生产实现：链上读节点（`CHAIN_CONFIG`）+ 本机 forum-sponsor（`SPONSOR_URL`、`SPONSOR_API_TOKEN`）。
pub struct Live {
    rpc: FailoverRpc,
    escrow: Address,
    token: Address,
    client: reqwest::Client,
    proof_url: String,
    sponsor_token: String,
}

#[derive(Deserialize)]
struct IssuedProof {
    proof: IssuedTerms,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IssuedTerms {
    pay_deadline: u64,
}

impl Live {
    /// 三项都配置了才结算拍卖；都没配置返回空（启动时警告），只配了一部分拒绝启动。
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        let get = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let (chain, url, token) = (
            get("CHAIN_CONFIG"),
            get("SPONSOR_URL"),
            get("SPONSOR_API_TOKEN"),
        );
        let (chain, url, token) = match (chain, url, token) {
            (Some(c), Some(u), Some(t)) => (c, u, t),
            (None, None, None) => return Ok(None),
            _ => anyhow::bail!(
                "auction settlement needs CHAIN_CONFIG, SPONSOR_URL and SPONSOR_API_TOKEN together"
            ),
        };
        let cfg = forum_chain::config::ChainConfig::load(chain.as_ref())?;
        Self::new(
            FailoverRpc::from_config(&cfg)?,
            cfg.contracts.escrow,
            cfg.token.address,
            &url,
            token,
        )
        .map(Some)
    }

    /// `sponsor_url` 形如 `http://127.0.0.1:8090`；只走本机，不经 `HTTP_PROXY`。
    pub fn new(
        rpc: FailoverRpc,
        escrow: Address,
        token: Address,
        sponsor_url: &str,
        sponsor_token: String,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            rpc,
            escrow,
            token,
            client: reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(30))
                .build()?,
            proof_url: format!("{}/auction/proof", sponsor_url.trim_end_matches('/')),
            sponsor_token,
        })
    }
}

#[async_trait::async_trait]
impl Settlement for Live {
    async fn usdt_balance(&self, wallet: &[u8]) -> anyhow::Result<Usdt> {
        let owner = Address::try_from(wallet).context("wallet address")?;
        let v = forum_chain::auction::token_balance(&self.rpc, self.token, owner).await?;
        Ok(Usdt(u128::try_from(v).unwrap_or(u128::MAX)))
    }

    async fn settled_on_chain(&self, seller: &[u8], auction_id: i64) -> anyhow::Result<bool> {
        let seller = Address::try_from(seller).context("seller address")?;
        let id = U256::from(u64::try_from(auction_id)?);
        Ok(forum_chain::auction::auction_settled(&self.rpc, self.escrow, seller, id).await?)
    }

    async fn issue_proof(
        &self,
        auction_id: i64,
        round: i16,
        bid_id: i64,
    ) -> anyhow::Result<OffsetDateTime> {
        let res = self
            .client
            .post(&self.proof_url)
            .bearer_auth(&self.sponsor_token)
            .json(&json!({ "auction_id": auction_id, "round": round, "bid_id": bid_id }))
            .send()
            .await
            .context("forum-sponsor unreachable")?;
        let status = res.status();
        if !status.is_success() {
            let body = res.text().await.unwrap_or_default();
            anyhow::bail!("forum-sponsor refused the proof ({status}): {body}");
        }
        let p: IssuedProof = res.json().await.context("forum-sponsor proof response")?;
        Ok(OffsetDateTime::from_unix_timestamp(i64::try_from(
            p.proof.pay_deadline,
        )?)?)
    }
}

/// 跑一轮，返回处理的拍卖数（只用于日志）。链上读取或 forum-sponsor 出错的那场只记日志，下一轮重试。
pub async fn run_once(
    db: &PgPool,
    s: &dyn Settlement,
    cfg: Config,
    push: Option<&Notify>,
) -> Result<usize, sqlx::Error> {
    let mut n = 0;
    for id in db::due_ended(db, BATCH).await? {
        match ended(db, s, id, push).await {
            Ok(()) => n += 1,
            Err(e) => tracing::warn!(auction = id, error = %e, "auction settlement failed"),
        }
    }
    for id in db::due_payment(db, cfg.grace, BATCH).await? {
        match overdue(db, s, id, cfg, push).await {
            Ok(true) => n += 1,
            Ok(false) => {}
            Err(e) => tracing::warn!(auction = id, error = %e, "auction default handling failed"),
        }
    }
    Ok(n)
}

/// 结束：流拍，或给第一位符合资格的出价者发证明。
async fn ended(
    db: &PgPool,
    s: &dyn Settlement,
    id: i64,
    push: Option<&Notify>,
) -> anyhow::Result<()> {
    let Some(a) = db::by_id(db, id).await? else {
        return Ok(());
    };
    if a.state != db::STATE_LIVE || a.ends_at.is_none_or(|e| e > OffsetDateTime::now_utc()) {
        return Ok(());
    }
    if !a.reserve_met() {
        return unsold(db, &a, push).await;
    }
    offer_next(db, s, &a, 1, push).await
}

/// 付款期已过：链上确实没成交就记违约，再递补或流拍。返回是否处理了（链上已成交时等 indexer）。
async fn overdue(
    db: &PgPool,
    s: &dyn Settlement,
    id: i64,
    cfg: Config,
    push: Option<&Notify>,
) -> anyhow::Result<bool> {
    let Some(a) = db::by_id(db, id).await? else {
        return Ok(false);
    };
    let Some(proof) = db::proof(db, a.id, a.round).await? else {
        anyhow::bail!("awaiting payment without a proof for round {}", a.round);
    };
    if a.state != db::STATE_AWAITING_PAYMENT
        || proof.pay_deadline + cfg.grace > OffsetDateTime::now_utc()
    {
        return Ok(false);
    }
    if s.settled_on_chain(&a.seller, a.id).await? {
        tracing::info!(
            auction = a.id,
            "auction settled on chain, waiting for the indexer"
        );
        return Ok(false);
    }
    let settings = forum_db::settings::auction(db).await?;
    let mut tx = db.begin().await?;
    if db::record_default(&mut tx, &a, &proof, settings.default_ban_secs)
        .await?
        .is_some()
    {
        tracing::info!(
            auction = a.id,
            round = a.round,
            user = proof.bidder_id,
            "auction winner defaulted"
        );
    }
    tx.commit().await?;
    if a.round < MAX_ROUNDS {
        offer_next(db, s, &a, a.round + 1, push).await?;
    } else {
        unsold(db, &a, push).await?;
    }
    Ok(true)
}

/// 出价人在拍卖结束（或递补）时是否仍符合资格；不符合时给出跳过原因。
async fn disqualified(
    db: &PgPool,
    s: &dyn Settlement,
    a: &Auction,
    c: &db::Candidate,
) -> anyhow::Result<Option<&'static str>> {
    let kinds = [sanctions::KIND_AUCTION_BAN, sanctions::KIND_BAN];
    if sanctions::active_kind(db, c.bidder_id, &kinds, Some(a.forum_id))
        .await?
        .is_some()
    {
        return Ok(Some("banned"));
    }
    let Some(wallet) = forum_db::wallet::wallet_on(db, c.bidder_id, a.chain_id).await? else {
        return Ok(Some("no_wallet"));
    };
    let eligibility = Eligibility::from_db(a.eligibility, a.eligibility_value.as_deref())
        .context("stored eligibility")?;
    if let Some(required) = eligibility.required(Usdt(c.amount_usdt.parse()?))
        && s.usdt_balance(&wallet.address).await? < required
    {
        return Ok(Some("balance"));
    }
    Ok(None)
}

/// 发第 `round` 份证明：从剩下的出价者里挑第一位符合资格的；没有人了就流拍。
async fn offer_next(
    db: &PgPool,
    s: &dyn Settlement,
    a: &Auction,
    round: i16,
    push: Option<&Notify>,
) -> anyhow::Result<()> {
    loop {
        let Some(c) = db::next_candidate(db, a.id).await? else {
            return unsold(db, a, push).await;
        };
        if let Some(reason) = disqualified(db, s, a, &c).await? {
            tracing::info!(
                auction = a.id,
                bidder = c.bidder_id,
                reason,
                "auction candidate passed over"
            );
            db::pass(db, a.id, c.bidder_id, reason).await?;
            continue;
        }
        let pay_deadline = s.issue_proof(a.id, round, c.bid_id).await?;
        let mut tx = db.begin().await?;
        if db::mark_offered(&mut *tx, a.id, round, pay_deadline).await? {
            let event = if round == 1 {
                db::EVENT_WON
            } else {
                db::EVENT_RUNNER_UP
            };
            let extra = json!({ "round": round, "price": c.amount, "pay_deadline": pay_deadline.unix_timestamp() });
            db::notify(&mut *tx, c.bidder_id, a, event, extra.clone()).await?;
            db::notify(&mut *tx, a.seller_id, a, db::EVENT_SOLD, extra).await?;
        }
        tx.commit().await?;
        publish(push, a, "awaiting_payment").await;
        return Ok(());
    }
}

async fn unsold(db: &PgPool, a: &Auction, push: Option<&Notify>) -> anyhow::Result<()> {
    let mut tx = db.begin().await?;
    if db::mark_unsold(&mut *tx, a.id, a.state).await? {
        db::notify(
            &mut *tx,
            a.seller_id,
            a,
            db::EVENT_UNSOLD,
            json!({ "bids": a.bid_count }),
        )
        .await?;
        tracing::info!(auction = a.id, "auction unsold");
    }
    tx.commit().await?;
    publish(push, a, "unsold").await;
    Ok(())
}

/// 拍卖状态变化的对外通知：主题页的实时连接与游客缓存（列表里的拍卖信息行）。
pub struct Notify {
    pub realtime: Realtime,
    pub guest: GuestCache,
}

/// 状态变了：游客缓存里带这条拍卖信息行的列表失效；订阅了拍卖帖主题页的连接重新取详情。
async fn publish(push: Option<&Notify>, a: &Auction, state: &str) {
    let (Some(n), Some(topic_id)) = (push, a.topic_id) else {
        return;
    };
    if let Err(err) = n
        .guest
        .bump(&guest_cache::listing(topic_id, &[a.forum_id]))
        .await
    {
        tracing::warn!(error = %err, "guest cache bump failed");
    }
    let rt = &n.realtime;
    let event = Event::AuctionState {
        topic_id,
        auction_id: a.id,
        state: state.to_owned(),
    };
    match Envelope::new(
        Route::Watchers {
            topic: Topic::Thread(topic_id),
        },
        &event,
    ) {
        Ok(e) => {
            if let Err(err) = rt.publish(&e).await {
                tracing::warn!(error = %err, "realtime publish failed");
            }
        }
        Err(err) => tracing::error!(error = %err, "event encoding failed"),
    }
}
