//! 免费代付提币次数（plan 第 5 节「gas 规则」）：只增流水 `withdraw_credit_ledger`，余额由插入触发器累加到
//! `withdraw_credits`。forum-api 发开通奖励，forum-indexer 发订单奖励，forum-sponsor 扣减与退回。

use sqlx::PgExecutor;
use time::OffsetDateTime;

/// 每人滚动 24 小时最多代付提币几次（有次数也不能超过）。
pub const DAILY_LIMIT: i64 = 3;

/// 开通钱包奖励 1 次，每人只发一次。返回这次是否发放。
pub async fn award_starter(db: impl PgExecutor<'_>, user_id: i64) -> Result<bool, sqlx::Error> {
    let r = sqlx::query!(
        "INSERT INTO withdraw_credit_ledger (user_id, delta, reason) VALUES ($1, 1, 'starter')
         ON CONFLICT DO NOTHING",
        user_id
    )
    .execute(db)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// 订单到达终态（已放款、已退款、已结算）且 finalized 后，买卖双方（本站用户）各奖励 1 次；
/// 状态不满足时什么都不做。每单每人只发一次，重放与区块回滚后重新折叠都不会重复。返回这次发放的人数。
pub async fn award_order(db: impl PgExecutor<'_>, order_id: i64) -> Result<u64, sqlx::Error> {
    let r = sqlx::query!(
        "INSERT INTO withdraw_credit_ledger (user_id, delta, reason, order_id)
         SELECT DISTINCT p.user_id, 1, 'order'::withdraw_credit_reason, o.id
         FROM escrow_orders o CROSS JOIN LATERAL (VALUES (o.buyer_id), (o.seller_id)) AS p (user_id)
         WHERE o.id = $1 AND o.state IN (3, 4, 5) AND o.level = 'finalized' AND p.user_id IS NOT NULL
         ON CONFLICT DO NOTHING",
        order_id
    )
    .execute(db)
    .await?;
    Ok(r.rows_affected())
}

/// 代付提币签名扣 1 次（与签名记录同一事务）。
pub async fn debit_withdraw(
    db: impl PgExecutor<'_>,
    user_id: i64,
    sponsorship_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO withdraw_credit_ledger (user_id, delta, reason, sponsorship_id)
         VALUES ($1, -1, 'withdraw', $2)",
        user_id,
        sponsorship_id
    )
    .execute(db)
    .await?;
    Ok(())
}

/// 同一账户同一 nonce 的签名（重签时用来认出待取代的旧签名）。
#[derive(Debug, Clone, Copy)]
pub struct Nonce<'a> {
    pub chain_id: i64,
    pub sender: &'a [u8],
    pub nonce: &'a str,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    /// 当前余额（重签时可短暂为负）。
    pub balance: i64,
    /// `since` 之后已用的代付提币次数（不含同一 nonce 待取代的旧签名）。
    pub today: i64,
    /// 同一 nonce 仍未结束的旧签名扣掉的次数：它们与新签名最多一笔能上链，没上链的过期后退回。
    pub held: i64,
}

pub async fn usage(
    db: impl PgExecutor<'_>,
    user_id: i64,
    since: OffsetDateTime,
    nonce: Option<Nonce<'_>>,
) -> Result<Usage, sqlx::Error> {
    sqlx::query_file_as!(
        Usage,
        "queries/withdraw_credits_usage.sql",
        user_id,
        since,
        nonce.map(|n| n.chain_id),
        nonce.map(|n| n.sender),
        nonce.map(|n| n.nonce)
    )
    .fetch_one(db)
    .await
}
