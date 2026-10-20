//! 拍卖成交证明（plan 第 6 节「成交」，ADR 0011）：forum-sponsor 用 `auctionSigner` 私钥签 `SettlementProof`。
//!
//! forum-worker 在拍卖结束（或上一位中标者违约）后经本机接口请求「第 `round` 份证明给出价 `bid_id`」。签名私钥只在
//! 这里，所以这里不信任请求本身，签名前从数据库独立核对：
//! - 拍卖在可以发这一份证明的状态：第 1 份要求进行中且结束时间已到；之后的每份要求上一份的付款截止时间已过、
//!   已记违约，且链上这场拍卖还没成交（`Escrow.auctionSettled`）；最多 [`MAX_ROUNDS`] 份。
//! - 卖家授权是上线时验过签的那份（重算哈希与库里一致）；`bid_id` 正是 [`forum_db::auctions::next_candidate`]
//!   选出的人（与 worker 同一条查询），出价不低于授权里的保留价，中标钱包是该用户登记的钱包。
//! - 付款截止时间 = 现在 + 站长设定的付款期（递补用递补期），截到授权有效期之内。
//!
//! 每份证明只签一次（`auction_proofs` 主键 + 咨询锁）：同一请求重试拿回同一份，换人则拒绝。
//! 私钥被盗的最坏情况见 plan 第 6 节：只能让某场拍卖以不低于保留价的价格卖给别人，钱是买家自己付的。

use std::sync::Arc;

use alloy::signers::SignerSync;
use alloy::signers::local::PrivateKeySigner;
use forum_core::ErrorCode;
use forum_core::auction::MAX_ROUNDS;
use forum_core::eip712::{Address, AuctionAuth, B256, Domain, SettlementProof, U256};
use forum_db::PgPool;
use forum_db::auctions::{self as db, Auction, Proof};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::chain::Chain;
use crate::jsonrpc::{Reject, Result};

/// forum-worker 的请求。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofRequest {
    pub auction_id: i64,
    pub round: i16,
    pub bid_id: i64,
}

/// 签好的证明。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssuedProof {
    pub auction_id: i64,
    pub round: i16,
    pub bid_id: i64,
    pub bidder_id: i64,
    pub proof: SettlementProof,
    /// `0x` 十六进制，65 字节。
    pub signature: String,
}

pub struct AuctionSigner {
    pub db: PgPool,
    pub chain: Arc<Chain>,
    /// 签名域：本链 + Escrow 合约（与卖家授权同一个域）。
    pub domain: Domain,
    signer: PrivateKeySigner,
}

fn unavailable(reason: impl Into<String>) -> Reject {
    Reject::new(ErrorCode::AuctionUnavailable, reason)
}

fn now_secs() -> u64 {
    u64::try_from(OffsetDateTime::now_utc().unix_timestamp()).unwrap_or(0)
}

fn bytes<const N: usize>(b: &[u8]) -> Result<[u8; N]> {
    Ok(b.try_into()
        .map_err(|_| anyhow::anyhow!("expected {N} bytes"))?)
}

fn amount(s: &str) -> Result<u128> {
    Ok(s.parse()
        .map_err(|_| anyhow::anyhow!("bad stored amount {s}"))?)
}

impl AuctionSigner {
    pub fn new(db: PgPool, chain: Arc<Chain>, domain: Domain, signer: PrivateKeySigner) -> Self {
        Self {
            db,
            chain,
            domain,
            signer,
        }
    }

    /// 链上 `EscrowConfig.auctionSigner`（启动时核对）。
    pub fn address(&self) -> alloy::primitives::Address {
        self.signer.address()
    }

    fn to_issued(p: &Proof) -> Result<IssuedProof> {
        Ok(IssuedProof {
            auction_id: p.auction_id,
            round: p.round,
            bid_id: p.bid_id,
            bidder_id: p.bidder_id,
            proof: SettlementProof {
                auth_hash: B256(bytes(&p.auth_hash)?),
                winner: Address(bytes(&p.winner)?),
                price: U256::from_u128(amount(&p.price)?),
                pay_deadline: u64::try_from(p.pay_deadline.unix_timestamp())
                    .map_err(anyhow::Error::from)?,
            },
            signature: alloy::hex::encode_prefixed(&p.signature),
        })
    }

    /// 上线时验过签的授权：重算哈希必须与库里一致，且属于这场拍卖、这个卖家、这个签名域。
    fn signed_auth(&self, a: &Auction) -> Result<(AuctionAuth, B256)> {
        let (Some(auth), Some(hash), Some(_)) = (
            a.auth.as_ref(),
            a.auth_hash.as_deref(),
            a.seller_signature.as_ref(),
        ) else {
            return Err(unavailable("auction has no signed authorization").into());
        };
        let auth: AuctionAuth =
            serde_json::from_value(auth.clone()).map_err(anyhow::Error::from)?;
        let hash = B256(bytes(hash)?);
        let id = u128::try_from(a.id).map_err(anyhow::Error::from)?;
        if auth.hash(&self.domain) != hash
            || auth.auction_id != U256::from_u128(id)
            || auth.seller.0.as_slice() != a.seller.as_slice()
            || a.verifying_contract.as_slice() != self.domain.verifying_contract.0.as_slice()
            || u64::try_from(a.chain_id).ok() != Some(self.domain.chain_id)
        {
            return Err(
                anyhow::anyhow!("auction {} authorization does not match its hash", a.id).into(),
            );
        }
        Ok((auth, hash))
    }

    /// 签发（或取回已签发的）第 `round` 份证明。
    pub async fn issue(&self, req: ProofRequest) -> Result<IssuedProof> {
        if !(1..=MAX_ROUNDS).contains(&req.round) {
            return Err(Reject::invalid("round out of range").into());
        }
        let a = db::by_id(&self.db, req.auction_id)
            .await?
            .ok_or_else(|| Reject::new(ErrorCode::NotFound, "no such auction"))?;
        if let Some(p) = db::proof(&self.db, a.id, req.round).await? {
            return if p.bid_id == req.bid_id {
                Self::to_issued(&p)
            } else {
                Err(unavailable("this round was already issued to another bid").into())
            };
        }
        let (auth, auth_hash) = self.signed_auth(&a)?;
        // 链上已经成交（indexer 可能还没跟上）：不再发证明。读链放在事务与锁之外。
        let settled = forum_chain::auction::auction_settled(
            &self.chain.rpc,
            self.domain.verifying_contract.0.into(),
            auth.seller.0.into(),
            alloy::primitives::U256::from_be_bytes(auth.auction_id.0),
        )
        .await?;
        if settled {
            return Err(unavailable("auction already settled on chain").into());
        }

        let mut tx = self.db.begin().await?;
        forum_db::sponsor::lock(&mut tx, &format!("auction-proof:{}", a.id)).await?;
        // 锁内重读：状态、轮次与候选人以此为准。
        let a = db::by_id(&mut *tx, a.id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("auction vanished"))?;
        if let Some(p) = db::proof(&mut *tx, a.id, req.round).await? {
            return if p.bid_id == req.bid_id {
                Self::to_issued(&p)
            } else {
                Err(unavailable("this round was already issued to another bid").into())
            };
        }
        let now = OffsetDateTime::now_utc();
        if req.round != a.round + 1 {
            return Err(unavailable(format!("auction is at round {}", a.round)).into());
        }
        if req.round == 1 {
            if a.state != db::STATE_LIVE || a.ends_at.is_none_or(|e| e > now) {
                return Err(unavailable("auction has not ended").into());
            }
        } else {
            let prev = db::proof(&mut *tx, a.id, req.round - 1)
                .await?
                .ok_or_else(|| anyhow::anyhow!("missing proof for previous round"))?;
            if a.state != db::STATE_AWAITING_PAYMENT
                || prev.pay_deadline >= now
                || !db::has_default(&mut *tx, a.id, req.round - 1).await?
            {
                return Err(unavailable("previous winner's payment window is still open").into());
            }
        }
        let c = db::next_candidate(&mut *tx, a.id)
            .await?
            .ok_or_else(|| unavailable("no eligible bid reaches the reserve"))?;
        if c.bid_id != req.bid_id {
            return Err(
                unavailable(format!("bid {} is not the next candidate", req.bid_id)).into(),
            );
        }
        let price = amount(&c.amount)?;
        if U256::from_u128(price) < auth.reserve_price {
            return Err(unavailable("bid below the signed reserve").into());
        }
        let wallet = forum_db::wallet::wallet_on(&mut *tx, c.bidder_id, a.chain_id)
            .await?
            .ok_or_else(|| unavailable("winner has no wallet on this chain"))?;
        let settings = forum_db::settings::auction(&mut *tx).await?;
        let pay_deadline =
            (now_secs() + u64::from(settings.pay_window(req.round))).min(auth.expiry);
        if pay_deadline <= now_secs() {
            return Err(unavailable("seller authorization expired").into());
        }
        let proof = SettlementProof {
            auth_hash,
            winner: Address(bytes(&wallet.address)?),
            price: U256::from_u128(price),
            pay_deadline,
        };
        // auctionSigner 是 EOA：合约对 hashProof 的摘要直接做 ecrecover（不加 eth_sign 前缀）。
        let digest = proof.hash(&self.domain);
        let signature = self
            .signer
            .sign_hash_sync(&digest.0.into())
            .map_err(anyhow::Error::from)?
            .as_bytes()
            .to_vec();
        let row = Proof {
            auction_id: a.id,
            round: req.round,
            bid_id: c.bid_id,
            bidder_id: c.bidder_id,
            auth_hash: auth_hash.0.to_vec(),
            winner: wallet.address.clone(),
            price: c.amount.clone(),
            pay_deadline: OffsetDateTime::from_unix_timestamp(
                i64::try_from(pay_deadline).map_err(anyhow::Error::from)?,
            )
            .map_err(anyhow::Error::from)?,
            signature,
        };
        if !db::insert_proof(&mut *tx, &row).await? {
            return Err(anyhow::anyhow!("proof row appeared under the lock").into());
        }
        tx.commit().await?;
        tracing::info!(
            auction = a.id,
            round = req.round,
            bid = c.bid_id,
            "auction settlement proof signed"
        );
        Self::to_issued(&row)
    }
}
