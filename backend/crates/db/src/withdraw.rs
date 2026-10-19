//! 提币地址（plan 第 5 节「提币地址冷却」）。生效与否按数据库时间判断，forum-api 展示与 forum-sponsor 代付用同一个时钟。

use sqlx::{PgExecutor, Postgres, Transaction};
use time::OffsetDateTime;

/// 每人最多保存的未撤销地址数。
pub const MAX_LIVE: i64 = 10;
/// 列表最多返回几条（未撤销的最多 [`MAX_LIVE`] 条，其余是最近撤销的）。
pub const LIST_LIMIT: i64 = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WithdrawAddress {
    pub id: i64,
    pub address: Vec<u8>,
    pub label: String,
    pub active_at: OffsetDateTime,
    pub revoked_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    /// 读取时已过冷却期（撤销与否另看 `revoked_at`）。
    pub active: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct NewWithdrawAddress<'a> {
    pub user_id: i64,
    pub chain_id: i64,
    pub address: &'a [u8],
    pub label: &'a str,
    /// 冷却秒数；0 = 立即生效。
    pub cooldown_secs: i64,
}

/// 添加前的检查结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    /// 未撤销的地址数（所有链）。
    pub live: i64,
    /// 同一条链上已有这个未撤销的地址。
    pub duplicate: bool,
    /// 添加过任何地址（含已撤销的）：只有第一个地址免冷却。
    pub used_before: bool,
}

pub async fn list(
    db: impl PgExecutor<'_>,
    user_id: i64,
    chain_id: i64,
) -> Result<Vec<WithdrawAddress>, sqlx::Error> {
    sqlx::query_file_as!(
        WithdrawAddress,
        "queries/withdraw_addresses_list.sql",
        user_id,
        chain_id,
        LIST_LIMIT
    )
    .fetch_all(db)
    .await
}

/// 已生效的地址（20 字节）。
pub async fn active_addresses(
    db: impl PgExecutor<'_>,
    user_id: i64,
    chain_id: i64,
) -> Result<Vec<Vec<u8>>, sqlx::Error> {
    let rows = sqlx::query_file!("queries/withdraw_addresses_active.sql", user_id, chain_id)
        .fetch_all(db)
        .await?;
    Ok(rows.into_iter().map(|r| r.address).collect())
}

/// 同一用户的添加互斥到事务结束：名额与「第一个地址」的判断不会被并发请求绕过。
pub async fn lock_user(
    tx: &mut Transaction<'_, Postgres>,
    user_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "SELECT pg_advisory_xact_lock(hashtextextended('withdraw-address:' || $1::bigint::text, 0))",
        user_id
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn stats(
    db: impl PgExecutor<'_>,
    user_id: i64,
    chain_id: i64,
    address: &[u8],
) -> Result<Stats, sqlx::Error> {
    sqlx::query_file_as!(
        Stats,
        "queries/withdraw_addresses_stats.sql",
        user_id,
        chain_id,
        address
    )
    .fetch_one(db)
    .await
}

pub async fn insert(
    db: impl PgExecutor<'_>,
    a: NewWithdrawAddress<'_>,
) -> Result<WithdrawAddress, sqlx::Error> {
    sqlx::query_as!(
        WithdrawAddress,
        r#"INSERT INTO withdraw_addresses (user_id, chain_id, address, label, active_at)
           VALUES ($1, $2, $3, $4, now() + make_interval(secs => $5::bigint::float8))
           RETURNING id, address, label, active_at, revoked_at, created_at, active_at <= now() AS "active!""#,
        a.user_id,
        a.chain_id,
        a.address,
        a.label,
        a.cooldown_secs
    )
    .fetch_one(db)
    .await
}

/// 撤销本人的一个未撤销地址（冷却中或已生效）；不存在、不是本人的或已撤销返回 `false`。
pub async fn revoke(db: impl PgExecutor<'_>, user_id: i64, id: i64) -> Result<bool, sqlx::Error> {
    let done = sqlx::query!(
        "UPDATE withdraw_addresses SET revoked_at = now() WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL",
        id,
        user_id
    )
    .execute(db)
    .await?;
    Ok(done.rows_affected() == 1)
}
