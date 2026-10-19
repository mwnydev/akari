//! 代付签名（ERC-7677 `pm_getPaymasterStubData` / `pm_getPaymasterData`）与 gas 估算。
//!
//! 签名前依次检查：目标 EntryPoint 与链 → sender 是本人钱包 → 部署工厂在白名单 → callData 白名单（含提币地址）→
//! 暂停 / 待上链 / 每单额度 / 代付提币次数 → gas 上限 → 模拟。签名 2 分钟有效（`validAfter` = 当前时间），绑定这一笔操作。
//! 额度、待上链与提币次数检查、写入签名记录与扣提币次数在同一事务里、按账户、用户和订单加咨询锁，
//! 并发请求不会多拿免费次数。

use std::sync::Arc;

use alloy::primitives::{Address, Bytes, U64, U256};
use alloy::signers::SignerSync;
use alloy::signers::local::PrivateKeySigner;
use forum_chain::aa::{SponsorTerms, UserOperation};
use forum_chain::config::AaConfig;
use forum_core::ErrorCode;
use forum_db::PgPool;
use forum_db::credits::{self, Nonce};
use forum_db::sponsor::{self as db, FreeCalls, NewCall, NewSponsorship, SponsorshipStatus};
use serde::Serialize;
use sqlx::PgConnection;
use time::OffsetDateTime;

use crate::chain::Chain;
use crate::config::{MAX_SPONSOR_FEE, Policy, VALIDITY_SECS};
use crate::jsonrpc::{Reject, Result};
use crate::kernel;
use crate::policy::{self, Action, Contracts, DAY_SECS, Facts, Priced, Tier};
use crate::sim::{self, GasEstimate, Limits, PM_POST_OP_GAS, PM_VERIFICATION_GAS, Sponsored};

/// 占位签名：估算阶段用，合约验签失败只返回 SIG_VALIDATION_FAILED，不收费。
const STUB_SIGNATURE: [u8; 65] = [0; 65];

/// 填进 UserOperation 的 Paymaster 字段（ERC-7677）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymasterFields {
    pub paymaster: Address,
    pub paymaster_data: Bytes,
    pub paymaster_verification_gas_limit: U256,
    pub paymaster_post_op_gas_limit: U256,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_final: Option<bool>,
}

pub struct Sponsor {
    pub db: PgPool,
    pub chain: Arc<Chain>,
    pub contracts: Contracts,
    pub aa: AaConfig,
    pub policy: Policy,
    signer: PrivateKeySigner,
    /// 代发热钱包（handleOps 的 beneficiary），估算 preVerificationGas 用。
    pub beneficiary: Address,
    /// 各 RPC 节点的模拟方法选择。
    pub sim: sim::Router,
}

/// 通过白名单后的一笔操作。
struct Evaluated {
    actions: Vec<Action>,
    /// 与 `actions` 对应：Escrow 订单是否已发货（只对 claimTimeout 有意义）。
    delivered: Vec<bool>,
    priced: Priced,
}

impl Evaluated {
    fn fee(&self, policy: &Policy) -> u128 {
        if self.priced.fee_due {
            policy.sponsor_fee
        } else {
            0
        }
    }
}

fn now_secs() -> u64 {
    u64::try_from(OffsetDateTime::now_utc().unix_timestamp()).unwrap_or(0)
}

fn sender_key(chain_id: u64, sender: Address) -> String {
    format!("sender:{chain_id}:{sender}")
}

/// 提币次数按用户计（不分链），不同链上的钱包也要互斥。
fn credits_key(user_id: i64) -> String {
    format!("credits:{user_id}")
}

impl Sponsor {
    pub fn new(
        db: PgPool,
        chain: Arc<Chain>,
        contracts: Contracts,
        aa: AaConfig,
        policy: Policy,
        signer: PrivateKeySigner,
        beneficiary: Address,
    ) -> Self {
        Self {
            db,
            chain,
            contracts,
            aa,
            policy,
            signer,
            beneficiary,
            sim: sim::Router::default(),
        }
    }

    pub fn chain_id(&self) -> u64 {
        self.chain.chain_id
    }

    /// Paymaster 签名地址（启动时与链上 `signer()` 核对）。
    pub fn signer_address(&self) -> Address {
        self.signer.address()
    }

    pub fn check_entry_point(&self, entry_point: Address) -> Result<()> {
        if entry_point != self.aa.entry_point {
            return Err(Reject::invalid("unsupported entryPoint").into());
        }
        Ok(())
    }

    fn check_target(&self, entry_point: Address, chain_id: U64) -> Result<()> {
        self.check_entry_point(entry_point)?;
        if chain_id != U64::from(self.chain_id()) {
            return Err(Reject::invalid("unsupported chainId").into());
        }
        Ok(())
    }

    /// 只给本人登记的钱包代付（plan 第 5 节）。
    async fn check_wallet(&self, user_id: i64, sender: Address) -> Result<()> {
        let chain_id = i64::try_from(self.chain_id()).map_err(anyhow::Error::from)?;
        let wallet = forum_db::wallet::wallet_on(&self.db, user_id, chain_id).await?;
        match wallet {
            None => Err(Reject::new(ErrorCode::WalletRequired, "no wallet on this chain").into()),
            Some(w) if w.address.as_slice() != sender.as_slice() => {
                Err(Reject::not_allowed("sender is not the user's wallet").into())
            }
            Some(_) => Ok(()),
        }
    }

    /// 用户已生效的提币地址：USDT 只代付转到这些地址（plan 第 5 节「提币地址冷却」）。
    async fn withdraw_addresses(&self, user_id: i64) -> Result<Vec<Address>> {
        let chain_id = i64::try_from(self.chain_id()).map_err(anyhow::Error::from)?;
        let rows = forum_db::withdraw::active_addresses(&self.db, user_id, chain_id).await?;
        Ok(rows
            .iter()
            .filter_map(|a| Address::try_from(a.as_slice()).ok())
            .collect())
    }

    fn check_factory(&self, op: &UserOperation) -> Result<()> {
        match op.factory.filter(|f| !f.is_zero()) {
            Some(f) if !self.aa.factories.contains(&f) => {
                Err(Reject::not_allowed("factory not allowed").into())
            }
            _ => Ok(()),
        }
    }

    /// Escrow `claimTimeout` 要看链上 latest 状态是否已发货。
    async fn delivered(&self, actions: &[Action]) -> Result<Vec<bool>> {
        let mut out = Vec::with_capacity(actions.len());
        for a in actions {
            let needs_state = matches!(a, Action::Order { market, method, .. }
                if policy::tier(*market, *method) == Tier::FreeIfDelivered);
            let delivered = match a {
                Action::Order {
                    contract, order_id, ..
                } if needs_state => {
                    let id = u64::try_from(*order_id)
                        .map_err(|_| Reject::invalid("order id out of range"))?;
                    self.chain
                        .escrow_delivered(*contract, id)
                        .await?
                        .ok_or_else(|| {
                            Reject::new(ErrorCode::SimulationFailed, "order not found")
                        })?
                }
                _ => false,
            };
            out.push(delivered);
        }
        Ok(out)
    }

    /// 各项免费额度已用掉的次数。
    async fn used(
        &self,
        conn: &mut PgConnection,
        sender: Address,
        actions: &[Action],
        delivered: &[bool],
    ) -> Result<Vec<Facts>> {
        let chain_id = i64::try_from(self.chain_id()).map_err(anyhow::Error::from)?;
        let since = OffsetDateTime::now_utc() - time::Duration::seconds(DAY_SECS);
        let mut out = Vec::with_capacity(actions.len());
        for (a, &delivered) in actions.iter().zip(delivered) {
            let used = match a {
                Action::Order {
                    contract,
                    market,
                    method,
                    order_id,
                } => match policy::tier(*market, *method) {
                    Tier::Free(Some(q)) => {
                        let order_id = order_id.to_string();
                        db::count_free_calls(
                            &mut *conn,
                            FreeCalls {
                                chain_id,
                                contract: contract.as_slice(),
                                order_id: &order_id,
                                method: method.name(),
                                sender: q.per_side.then_some(sender.as_slice()),
                                since: q.per_day.then_some(since),
                            },
                        )
                        .await?
                    }
                    _ => 0,
                },
                _ => 0,
            };
            out.push(Facts { used, delivered });
        }
        Ok(out)
    }

    /// 暂停与「每个账户同时只有一笔待上链」。同一 nonce 的未提交签名可以重签（取代旧的）。
    async fn check_account(
        &self,
        conn: &mut PgConnection,
        user_id: i64,
        op: &UserOperation,
    ) -> Result<()> {
        if let Some(until) = db::suspended_until(&mut *conn, user_id).await? {
            return Err(Reject::new(ErrorCode::SponsorSuspended, format!("until {until}")).into());
        }
        let chain_id = i64::try_from(self.chain_id()).map_err(anyhow::Error::from)?;
        let nonce = op.nonce.to_string();
        let pending = db::pending_for_sender(&mut *conn, chain_id, op.sender.as_slice()).await?;
        let blocking = pending
            .iter()
            .any(|p| p.nonce != nonce || p.status == SponsorshipStatus::Submitted);
        if blocking {
            return Err(Reject::new(
                ErrorCode::SponsorPending,
                "a sponsored operation is pending",
            )
            .into());
        }
        Ok(())
    }

    /// 提回交易所要有代付提币次数，且不超过每天的上限。
    async fn check_credits(
        &self,
        conn: &mut PgConnection,
        user_id: i64,
        op: &UserOperation,
    ) -> Result<()> {
        let chain_id = i64::try_from(self.chain_id()).map_err(anyhow::Error::from)?;
        let since = OffsetDateTime::now_utc() - time::Duration::seconds(DAY_SECS);
        let nonce = op.nonce.to_string();
        let u = credits::usage(
            &mut *conn,
            user_id,
            since,
            Some(Nonce {
                chain_id,
                sender: op.sender.as_slice(),
                nonce: &nonce,
            }),
        )
        .await?;
        policy::check_withdraw(u.balance + u.held, u.today)?;
        Ok(())
    }

    /// 白名单 + 额度定价（只读）。
    async fn evaluate(&self, user_id: i64, op: &UserOperation) -> Result<Evaluated> {
        self.check_wallet(user_id, op.sender).await?;
        self.check_factory(op)?;
        let calls = kernel::decode(&op.call_data)?;
        let withdraw_to = self.withdraw_addresses(user_id).await?;
        let actions = policy::classify(&calls, op.sender, &self.contracts, &withdraw_to)?;
        let delivered = self.delivered(&actions).await?;
        let mut conn = self.db.acquire().await?;
        self.check_account(&mut conn, user_id, op).await?;
        let facts = self
            .used(&mut conn, op.sender, &actions, &delivered)
            .await?;
        let priced = policy::price(&actions, &facts)?;
        if priced.withdraw {
            self.check_credits(&mut conn, user_id, op).await?;
        }
        Ok(Evaluated {
            actions,
            delivered,
            priced,
        })
    }

    fn fields(&self, paymaster_data: Bytes, is_final: Option<bool>) -> PaymasterFields {
        PaymasterFields {
            paymaster: self.aa.paymaster,
            paymaster_data,
            paymaster_verification_gas_limit: U256::from(PM_VERIFICATION_GAS),
            paymaster_post_op_gas_limit: U256::from(PM_POST_OP_GAS),
            is_final,
        }
    }

    fn with_paymaster(&self, op: &UserOperation, paymaster_data: Bytes) -> UserOperation {
        UserOperation {
            paymaster: Some(self.aa.paymaster),
            paymaster_verification_gas_limit: Some(U256::from(PM_VERIFICATION_GAS)),
            paymaster_post_op_gas_limit: Some(U256::from(PM_POST_OP_GAS)),
            paymaster_data: Some(paymaster_data),
            ..op.clone()
        }
    }

    fn sponsored(&self, terms: SponsorTerms) -> Sponsored {
        Sponsored {
            paymaster: self.aa.paymaster,
            token: self.contracts.token,
            terms,
        }
    }

    fn terms(&self, fee: u128) -> Result<SponsorTerms> {
        if fee > MAX_SPONSOR_FEE {
            return Err(anyhow::anyhow!("sponsor fee above hard cap").into());
        }
        let now = now_secs();
        Ok(SponsorTerms {
            valid_until: now + VALIDITY_SECS,
            valid_after: now,
            fee,
        })
    }

    /// `pm_getPaymasterStubData`：规则检查通过后给出费用与占位签名（不模拟、不记录）。
    pub async fn stub(
        &self,
        user_id: i64,
        op: &UserOperation,
        entry_point: Address,
        chain_id: U64,
    ) -> Result<PaymasterFields> {
        self.check_target(entry_point, chain_id)?;
        let ev = self.evaluate(user_id, op).await?;
        let data = self.terms(ev.fee(&self.policy))?.encode(&STUB_SIGNATURE)?;
        Ok(self.fields(data, Some(false)))
    }

    /// `eth_estimateUserOperationGas`：规则检查 + 模拟，按模拟结果给出 gas 字段。
    pub async fn estimate(
        &self,
        user_id: i64,
        op: &UserOperation,
        entry_point: Address,
    ) -> Result<GasEstimate> {
        self.check_entry_point(entry_point)?;
        let ev = self.evaluate(user_id, op).await?;
        let terms = self.terms(ev.fee(&self.policy))?;
        let stub = self.with_paymaster(op, terms.encode(&STUB_SIGNATURE)?);
        let s = sim::simulate(
            &self.chain,
            &self.sim,
            &stub,
            &self.sponsored(terms),
            Limits::Estimate,
        )
        .await?;
        sim::estimate(&self.chain, &stub, s, self.beneficiary).await
    }

    /// `pm_getPaymasterData`：完整检查、模拟、签名并记录。
    pub async fn sponsor(
        &self,
        user_id: i64,
        op: &UserOperation,
        entry_point: Address,
        chain_id: U64,
    ) -> Result<PaymasterFields> {
        self.check_target(entry_point, chain_id)?;
        let ev = self.evaluate(user_id, op).await?;
        let fee = ev.fee(&self.policy);
        let gas_price = self.chain.gas_price().await?;
        let signed_block = self.chain.block_number().await?;
        let mut signed = self.with_paymaster(op, Bytes::new());
        sim::check_limits(&signed, &self.policy, gas_price)?;
        let s = sim::simulate(
            &self.chain,
            &self.sim,
            &signed,
            &self.sponsored(self.terms(fee)?),
            Limits::Signed,
        )
        .await?;

        // 有效期从模拟之后算起。
        let terms = self.terms(fee)?;
        let hash = terms.hash(&signed, self.chain_id(), self.aa.paymaster)?;
        let signature = self
            .signer
            .sign_message_sync(hash.as_slice())
            .map_err(anyhow::Error::from)?;
        let data = terms.encode(&signature.as_bytes())?;
        signed.paymaster_data = Some(data.clone());
        let user_op_hash = signed.hash(self.aa.entry_point, self.chain_id())?;

        self.record(
            user_id,
            &signed,
            &ev,
            terms,
            user_op_hash.as_slice(),
            signed_block,
        )
        .await?;
        tracing::info!(
            user_id,
            sender = %op.sender,
            fee,
            %user_op_hash,
            sim_method = s.method.as_str(),
            "sponsorship signed"
        );
        Ok(self.fields(data, None))
    }

    /// 在锁内重新检查暂停、待上链与额度，再写入签名记录。额度在模拟之后被别的请求用掉时要求重试。
    /// 提回交易所在同一事务里检查并扣 1 次代付提币次数（同一 nonce 的旧签名先标为被取代，再数次数）。
    async fn record(
        &self,
        user_id: i64,
        op: &UserOperation,
        ev: &Evaluated,
        terms: SponsorTerms,
        user_op_hash: &[u8],
        signed_block: u64,
    ) -> Result<()> {
        let chain_id = i64::try_from(self.chain_id()).map_err(anyhow::Error::from)?;
        let mut tx = self.db.begin().await?;
        // 加锁顺序固定：账户 → 用户 → 订单（按键排序）。
        let mut keys = vec![sender_key(self.chain_id(), op.sender)];
        if ev.priced.withdraw {
            keys.push(credits_key(user_id));
        }
        let mut orders: Vec<String> = ev
            .priced
            .calls
            .iter()
            .map(|c| format!("order:{}:{}:{}", self.chain_id(), c.contract, c.order_id))
            .collect();
        orders.sort();
        orders.dedup();
        keys.extend(orders);
        for k in &keys {
            db::lock(&mut tx, k).await?;
        }
        self.check_account(&mut tx, user_id, op).await?;
        // 发货状态沿用模拟前查到的链上值，额度在锁内重新数。
        let facts = self
            .used(&mut tx, op.sender, &ev.actions, &ev.delivered)
            .await?;
        if policy::price(&ev.actions, &facts)? != ev.priced {
            return Err(Reject::new(ErrorCode::SponsorPending, "quota changed, retry").into());
        }
        let nonce = op.nonce.to_string();
        db::replace_signed(
            &mut *tx,
            chain_id,
            op.sender.as_slice(),
            &nonce,
            user_op_hash,
        )
        .await?;
        if ev.priced.withdraw {
            self.check_credits(&mut tx, user_id, op).await?;
        }
        let valid_until = OffsetDateTime::from_unix_timestamp(
            i64::try_from(terms.valid_until).map_err(anyhow::Error::from)?,
        )
        .map_err(anyhow::Error::from)?;
        let fee = terms.fee.to_string();
        let Some(id) = db::insert(
            &mut *tx,
            NewSponsorship {
                user_id,
                chain_id,
                sender: op.sender.as_slice(),
                nonce: &nonce,
                user_op_hash,
                fee: &fee,
                valid_until,
                signed_block: i64::try_from(signed_block).map_err(anyhow::Error::from)?,
            },
        )
        .await?
        else {
            // 与已有记录完全相同（同一秒内重签），额度已经记过。
            tx.commit().await?;
            return Ok(());
        };
        if ev.priced.withdraw {
            credits::debit_withdraw(&mut *tx, user_id, id).await?;
        }
        for c in &ev.priced.calls {
            let order_id = c.order_id.to_string();
            db::insert_call(
                &mut *tx,
                id,
                NewCall {
                    contract: c.contract.as_slice(),
                    order_id: &order_id,
                    method: c.method.name(),
                    free: c.free,
                },
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
}
