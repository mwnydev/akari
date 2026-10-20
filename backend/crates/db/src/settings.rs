//! 站长在后台修改的链下参数（plan 第 9 节）；每类参数一行 JSON。

use forum_core::auction::AuctionSettings;
use forum_core::staff::ModerationSettings;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use sqlx::PgExecutor;

const KEY_MODERATION: &str = "moderation";
const KEY_AUCTION: &str = "auction";

/// 读一类参数；没有设置过（或存的 JSON 读不懂）时用默认值。
async fn get<T: DeserializeOwned + Default>(
    db: impl PgExecutor<'_>,
    key: &str,
) -> Result<T, sqlx::Error> {
    let row = sqlx::query!("SELECT value FROM site_settings WHERE key = $1", key)
        .fetch_optional(db)
        .await?;
    Ok(row
        .and_then(|r| serde_json::from_value::<T>(r.value).ok())
        .unwrap_or_default())
}

async fn set<T: Serialize>(
    db: impl PgExecutor<'_>,
    key: &str,
    value: &T,
    by: i64,
) -> Result<(), sqlx::Error> {
    let json: Value = serde_json::to_value(value).map_err(|e| sqlx::Error::Encode(Box::new(e)))?;
    sqlx::query!(
        "INSERT INTO site_settings (key, value, updated_by) VALUES ($1, $2, $3)
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_by = EXCLUDED.updated_by,
                                         updated_at = now()",
        key,
        json,
        by
    )
    .execute(db)
    .await?;
    Ok(())
}

/// 管理操作限速与异常检测阈值；库里的值越界时收回到上下限内。
pub async fn moderation(db: impl PgExecutor<'_>) -> Result<ModerationSettings, sqlx::Error> {
    Ok(get::<ModerationSettings>(db, KEY_MODERATION)
        .await?
        .clamped())
}

/// 保存（调用方已校验范围）。
pub async fn set_moderation(
    db: impl PgExecutor<'_>,
    value: &ModerationSettings,
    by: i64,
) -> Result<(), sqlx::Error> {
    set(db, KEY_MODERATION, value, by).await
}

/// 拍卖参数与上线开关（默认关闭）；库里的值越界时收回到上下限内。
pub async fn auction(db: impl PgExecutor<'_>) -> Result<AuctionSettings, sqlx::Error> {
    Ok(get::<AuctionSettings>(db, KEY_AUCTION).await?.clamped())
}

/// 保存（调用方已校验范围）。
pub async fn set_auction(
    db: impl PgExecutor<'_>,
    value: &AuctionSettings,
    by: i64,
) -> Result<(), sqlx::Error> {
    set(db, KEY_AUCTION, value, by).await
}
