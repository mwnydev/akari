//! forum-sponsor 的代付记录（plan 第 5 节「gas 规则」）。只有 forum-sponsor 调用。
//!
//! 一笔代付签名 = `sponsorships` 一行；其中的担保合约调用写入 `sponsored_calls`，用来数每单的免费代付次数。
//! nonce、金额是 `NUMERIC`，进出都用十进制字符串。

use serde_json::Value;
use sqlx::{PgExecutor, Postgres, Transaction};
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "sponsorship_status", rename_all = "lowercase")]
pub enum SponsorshipStatus {
    /// 已签代付，还没收到 `eth_sendUserOperation`
    Signed,
    /// 同一 nonce 又签了一次；旧签名仍可能被别处提交，过期后照样对账
    Replaced,
    /// 已进代发队列或已转交备用 bundler
    Submitted,
    /// 已上链，callData 成功
    Included,
    /// 已上链，callData 回滚（代付费照收，gas 照付）
    Failed,
    /// 签名过期且没有上链
    Expired,
    /// 代发前验证不通过，没有代发；签名过期前仍可能经别处上链，过期后由对账改为链上结果或 `Expired`
    Dropped,
}

/// 在事务内串行化同一个键（账户、订单）的代付审批，事务结束自动释放。
pub async fn lock(tx: &mut Transaction<'_, Postgres>, key: &str) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "SELECT pg_advisory_xact_lock(hashtextextended('forum-sponsor:' || $1, 0))",
        key
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// 仍在暂停期内时返回截止时间。
pub async fn suspended_until(
    db: impl PgExecutor<'_>,
    user_id: i64,
) -> Result<Option<OffsetDateTime>, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT suspended_until AS "until!" FROM sponsor_accounts
           WHERE user_id = $1 AND suspended_until > now()"#,
        user_id
    )
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| r.until))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub id: i64,
    pub nonce: String,
    pub status: SponsorshipStatus,
}

/// 该账户还可能上链的代付：未过期的签名，以及已提交但还没对账的操作。
pub async fn pending_for_sender(
    db: impl PgExecutor<'_>,
    chain_id: i64,
    sender: &[u8],
) -> Result<Vec<Pending>, sqlx::Error> {
    sqlx::query_as!(
        Pending,
        r#"SELECT id, nonce::text AS "nonce!", status AS "status: SponsorshipStatus"
           FROM sponsorships
           WHERE sender = $1 AND chain_id = $2
             AND (status = 'submitted' OR (status = 'signed' AND valid_until > now()))
           ORDER BY id"#,
        sender,
        chain_id
    )
    .fetch_all(db)
    .await
}

/// 同一账户、同一 nonce 的其他旧签名改为 `replaced`（新签名取代它，两者最多一笔能上链）。
pub async fn replace_signed(
    db: impl PgExecutor<'_>,
    chain_id: i64,
    sender: &[u8],
    nonce: &str,
    keep_hash: &[u8],
) -> Result<u64, sqlx::Error> {
    let r = sqlx::query!(
        "UPDATE sponsorships SET status = 'replaced', updated_at = now()
         WHERE sender = $1 AND chain_id = $2 AND nonce = $3::text::numeric AND status = 'signed'
           AND user_op_hash <> $4",
        sender,
        chain_id,
        nonce,
        keep_hash
    )
    .execute(db)
    .await?;
    Ok(r.rows_affected())
}

#[derive(Debug, Clone, Copy)]
pub struct FreeCalls<'a> {
    pub chain_id: i64,
    pub contract: &'a [u8],
    pub order_id: &'a str,
    pub method: &'a str,
    /// 只数这个账户的（「每方各 1 次」）；`None` 数整单。
    pub sender: Option<&'a [u8]>,
    /// 只数这之后签的（「每天 1 次」）；`None` 不限时间。
    pub since: Option<OffsetDateTime>,
}

/// 已用掉的免费代付次数。对账确认过期未上链的签名不算；被取代、代发前作废的签名在此之前仍可能经别处上链，照算
/// （宁可少给一次）。同一 nonce 的多笔签名最多一笔能上链，只算一次。
pub async fn count_free_calls(
    db: impl PgExecutor<'_>,
    q: FreeCalls<'_>,
) -> Result<i64, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT count(DISTINCT (s.sender, s.nonce)) AS "n!" FROM sponsored_calls c
           JOIN sponsorships s ON s.id = c.sponsorship_id
           WHERE c.contract = $1 AND c.order_id = $2::text::numeric AND c.method = $3 AND c.free
             AND s.chain_id = $4 AND s.status <> 'expired'
             AND ($5::bytea IS NULL OR s.sender = $5)
             AND ($6::timestamptz IS NULL OR s.created_at > $6)"#,
        q.contract,
        q.order_id,
        q.method,
        q.chain_id,
        q.sender,
        q.since
    )
    .fetch_one(db)
    .await?;
    Ok(row.n)
}

#[derive(Debug, Clone, Copy)]
pub struct NewSponsorship<'a> {
    pub user_id: i64,
    pub chain_id: i64,
    pub sender: &'a [u8],
    pub nonce: &'a str,
    pub user_op_hash: &'a [u8],
    pub fee: &'a str,
    pub valid_until: OffsetDateTime,
    pub signed_block: i64,
}

/// 写入签名记录。同一秒内重签同一笔操作会得到完全相同的签名与哈希：已有记录时返回 `None`。
pub async fn insert(
    db: impl PgExecutor<'_>,
    s: NewSponsorship<'_>,
) -> Result<Option<i64>, sqlx::Error> {
    let row = sqlx::query!(
        "INSERT INTO sponsorships (user_id, chain_id, sender, nonce, user_op_hash, fee, valid_until, signed_block)
         VALUES ($1, $2, $3, $4::text::numeric, $5, $6::text::numeric, $7, $8)
         ON CONFLICT (user_op_hash) DO NOTHING
         RETURNING id",
        s.user_id,
        s.chain_id,
        s.sender,
        s.nonce,
        s.user_op_hash,
        s.fee,
        s.valid_until,
        s.signed_block
    )
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| r.id))
}

#[derive(Debug, Clone, Copy)]
pub struct NewCall<'a> {
    pub contract: &'a [u8],
    pub order_id: &'a str,
    pub method: &'a str,
    pub free: bool,
}

pub async fn insert_call(
    db: impl PgExecutor<'_>,
    sponsorship_id: i64,
    c: NewCall<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO sponsored_calls (sponsorship_id, contract, order_id, method, free)
         VALUES ($1, $2, $3::text::numeric, $4, $5)
         ON CONFLICT DO NOTHING",
        sponsorship_id,
        c.contract,
        c.order_id,
        c.method,
        c.free
    )
    .execute(db)
    .await?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sponsorship {
    pub id: i64,
    pub user_id: i64,
    pub chain_id: i64,
    pub sender: Vec<u8>,
    pub status: SponsorshipStatus,
    pub valid_until: OffsetDateTime,
    pub user_op: Option<Value>,
    pub via_fallback: bool,
    pub tx_hash: Option<Vec<u8>>,
    pub block_number: Option<i64>,
}

pub async fn by_hash(
    db: impl PgExecutor<'_>,
    user_op_hash: &[u8],
) -> Result<Option<Sponsorship>, sqlx::Error> {
    sqlx::query_as!(
        Sponsorship,
        r#"SELECT id, user_id, chain_id, sender, status AS "status: SponsorshipStatus", valid_until,
                  user_op, via_fallback, tx_hash, block_number
           FROM sponsorships WHERE user_op_hash = $1"#,
        user_op_hash
    )
    .fetch_optional(db)
    .await
}

/// 收到 `eth_sendUserOperation`：只有未过期的 `signed` 行能提交，且只能提交一次。
pub async fn mark_submitted(
    db: impl PgExecutor<'_>,
    id: i64,
    user_op: &Value,
    via_fallback: bool,
) -> Result<bool, sqlx::Error> {
    let r = sqlx::query!(
        "UPDATE sponsorships SET status = 'submitted', user_op = $2, via_fallback = $3, updated_at = now()
         WHERE id = $1 AND status = 'signed' AND valid_until > now()",
        id,
        user_op,
        via_fallback
    )
    .execute(db)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// 链上结果（`UserOperationEvent`）。
#[derive(Debug, Clone, Copy)]
pub struct Outcome<'a> {
    pub user_op_hash: &'a [u8],
    pub success: bool,
    pub tx_hash: &'a [u8],
    pub block_number: i64,
    pub actual_gas_cost: &'a str,
}

/// 写入链上结果。返回 `Some(user_id)` 表示这次把它记成了失败（调用方据此累计失败次数）；
/// 已经记过结果的行不重复处理，代发器与对账任务谁先看到都一样。
pub async fn settle(db: impl PgExecutor<'_>, o: Outcome<'_>) -> Result<Option<i64>, sqlx::Error> {
    let status = if o.success {
        SponsorshipStatus::Included
    } else {
        SponsorshipStatus::Failed
    };
    let row = sqlx::query!(
        "UPDATE sponsorships SET status = $2, tx_hash = $3, block_number = $4,
           actual_gas_cost = $5::text::numeric, updated_at = now()
         WHERE user_op_hash = $1 AND status NOT IN ('included', 'failed')
         RETURNING user_id",
        o.user_op_hash,
        status as SponsorshipStatus,
        o.tx_hash,
        o.block_number,
        o.actual_gas_cost
    )
    .fetch_optional(db)
    .await?;
    Ok(row.filter(|_| !o.success).map(|r| r.user_id))
}

/// 标记没有上链：`dropped`（代发前验证不通过）或 `expired`（签名已过期，对账没找到）。已有链上结果的行不动。
/// `dropped` 的签名过期前仍可能经第三方上链，免费额度与代付提币次数都不退，留给对账；
/// `expired` 才释放免费额度、退回这笔签名扣掉的代付提币次数（只在这次真的结束它时退，退一次）。
pub async fn close(
    db: impl PgExecutor<'_>,
    user_op_hash: &[u8],
    status: SponsorshipStatus,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "WITH closed AS (
           UPDATE sponsorships SET status = $2, updated_at = now()
           WHERE user_op_hash = $1 AND status IN ('signed', 'replaced', 'submitted', 'dropped')
           RETURNING id, user_id, status
         )
         INSERT INTO withdraw_credit_ledger (user_id, delta, reason, sponsorship_id)
         SELECT c.user_id, 1, 'refund_expired', c.id FROM closed c
         JOIN withdraw_credit_ledger d ON d.sponsorship_id = c.id AND d.reason = 'withdraw'
         WHERE c.status = 'expired'
         ON CONFLICT DO NOTHING",
        user_op_hash,
        status as SponsorshipStatus
    )
    .execute(db)
    .await?;
    Ok(())
}

/// 链上失败计一次；窗口（从第一次失败算起 `window_secs`）内达到 `threshold` 次就暂停 `suspend_secs`，
/// 并清零重新计数。返回新的暂停截止时间。
pub async fn record_failure(
    tx: &mut Transaction<'_, Postgres>,
    user_id: i64,
    threshold: i32,
    window_secs: f64,
    suspend_secs: f64,
) -> Result<Option<OffsetDateTime>, sqlx::Error> {
    let row = sqlx::query!(
        "INSERT INTO sponsor_accounts AS a (user_id, failures, first_failure_at) VALUES ($1, 1, now())
         ON CONFLICT (user_id) DO UPDATE SET
           failures = CASE WHEN a.first_failure_at > now() - make_interval(secs => $2)
                           THEN a.failures + 1 ELSE 1 END,
           first_failure_at = CASE WHEN a.first_failure_at > now() - make_interval(secs => $2)
                                   THEN a.first_failure_at ELSE now() END,
           updated_at = now()
         RETURNING failures",
        user_id,
        window_secs
    )
    .fetch_one(&mut **tx)
    .await?;
    if row.failures < threshold {
        return Ok(None);
    }
    let row = sqlx::query!(
        r#"UPDATE sponsor_accounts
           SET suspended_until = now() + make_interval(secs => $2), failures = 0, first_failure_at = NULL,
               updated_at = now()
           WHERE user_id = $1
           RETURNING suspended_until AS "until!""#,
        user_id,
        suspend_secs
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(Some(row.until))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Due {
    pub user_op_hash: Vec<u8>,
    pub signed_block: i64,
}

/// 签名已过期超过 `grace_secs`、还没有结果的行（含代发前作废的）：结果已经确定（EntryPoint 不会再接受它），可以对账。
pub async fn due_for_reconcile(
    db: impl PgExecutor<'_>,
    grace_secs: f64,
    limit: i64,
) -> Result<Vec<Due>, sqlx::Error> {
    sqlx::query_as!(
        Due,
        r#"SELECT user_op_hash, signed_block FROM sponsorships
           WHERE status IN ('signed', 'replaced', 'submitted', 'dropped')
             AND valid_until < now() - make_interval(secs => $1)
           ORDER BY valid_until
           LIMIT $2"#,
        grace_secs,
        limit
    )
    .fetch_all(db)
    .await
}
