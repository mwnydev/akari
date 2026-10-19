//! 钱包 Passkey 与智能账户地址（plan 第 5 节「开通交易」）。

use serde_json::Value;
use sqlx::PgExecutor;
use time::OffsetDateTime;

/// `passkeys.purpose`：钱包签名。
pub const PURPOSE_WALLET: i16 = 1;

#[derive(Debug, Clone, PartialEq)]
pub struct Passkey {
    pub id: i64,
    pub credential_id: Vec<u8>,
    /// webauthn-rs 序列化的凭证（含签名计数），验证时取出。
    pub credential: Value,
    pub pubkey_x: Vec<u8>,
    pub pubkey_y: Vec<u8>,
    pub created_at: OffsetDateTime,
    pub last_used_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Copy)]
pub struct NewPasskey<'a> {
    pub user_id: i64,
    pub credential_id: &'a [u8],
    pub credential: &'a Value,
    pub public_key_cose: &'a [u8],
    pub pubkey_x: &'a [u8],
    pub pubkey_y: &'a [u8],
}

/// 保存新的钱包 Passkey；凭证 ID 已存在（同一个 Passkey 重复注册）返回 `None`。
pub async fn insert_passkey(
    db: impl PgExecutor<'_>,
    p: NewPasskey<'_>,
) -> Result<Option<i64>, sqlx::Error> {
    let row = sqlx::query!(
        r#"INSERT INTO passkeys (user_id, purpose, credential_id, credential, public_key_cose, pubkey_x, pubkey_y, last_used_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, now())
           ON CONFLICT (credential_id) DO NOTHING
           RETURNING id"#,
        p.user_id,
        PURPOSE_WALLET,
        p.credential_id,
        p.credential,
        p.public_key_cose,
        p.pubkey_x,
        p.pubkey_y
    )
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| r.id))
}

pub async fn wallet_passkeys(
    db: impl PgExecutor<'_>,
    user_id: i64,
) -> Result<Vec<Passkey>, sqlx::Error> {
    sqlx::query_as!(
        Passkey,
        r#"SELECT id, credential_id, credential, pubkey_x AS "pubkey_x!", pubkey_y AS "pubkey_y!", created_at, last_used_at
           FROM passkeys WHERE user_id = $1 AND purpose = $2 ORDER BY id"#,
        user_id,
        PURPOSE_WALLET
    )
    .fetch_all(db)
    .await
}

/// 验证成功后写回凭证（签名计数、备份状态可能变化）。
pub async fn touch_passkey(
    db: impl PgExecutor<'_>,
    id: i64,
    credential: &Value,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE passkeys SET credential = $2, last_used_at = now() WHERE id = $1",
        id,
        credential
    )
    .execute(db)
    .await?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wallet {
    pub id: i64,
    pub chain_id: i64,
    pub address: Vec<u8>,
    pub passkey_id: i64,
    /// 计算地址用的初始化参数版本（ADR 0008）。
    pub init_version: i32,
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Clone, Copy)]
pub struct NewWallet<'a> {
    pub user_id: i64,
    pub chain_id: i64,
    pub address: &'a [u8],
    pub passkey_id: i64,
    pub init_version: i32,
}

/// 登记钱包地址。每人每条链一个、每个地址只属于一人：冲突返回 `None`。
pub async fn insert_wallet(
    db: impl PgExecutor<'_>,
    w: NewWallet<'_>,
) -> Result<Option<Wallet>, sqlx::Error> {
    sqlx::query_as!(
        Wallet,
        r#"INSERT INTO wallets (user_id, chain_id, address, passkey_id, init_version) VALUES ($1, $2, $3, $4, $5)
           ON CONFLICT DO NOTHING
           RETURNING id, chain_id, address, passkey_id, init_version, created_at"#,
        w.user_id,
        w.chain_id,
        w.address,
        w.passkey_id,
        w.init_version
    )
    .fetch_optional(db)
    .await
}

pub async fn wallets(db: impl PgExecutor<'_>, user_id: i64) -> Result<Vec<Wallet>, sqlx::Error> {
    sqlx::query_as!(
        Wallet,
        "SELECT id, chain_id, address, passkey_id, init_version, created_at FROM wallets WHERE user_id = $1 ORDER BY chain_id",
        user_id
    )
    .fetch_all(db)
    .await
}

pub async fn wallet_on(
    db: impl PgExecutor<'_>,
    user_id: i64,
    chain_id: i64,
) -> Result<Option<Wallet>, sqlx::Error> {
    sqlx::query_as!(
        Wallet,
        "SELECT id, chain_id, address, passkey_id, init_version, created_at FROM wallets\n         WHERE user_id = $1 AND chain_id = $2",
        user_id,
        chain_id
    )
    .fetch_optional(db)
    .await
}

/// 地址属于哪个用户。
pub async fn owner_of(
    db: impl PgExecutor<'_>,
    chain_id: i64,
    address: &[u8],
) -> Result<Option<i64>, sqlx::Error> {
    let row = sqlx::query!(
        "SELECT user_id FROM wallets WHERE chain_id = $1 AND address = $2",
        chain_id,
        address
    )
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| r.user_id))
}
