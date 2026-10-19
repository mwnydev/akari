//! 站长在后台修改的链下参数（plan 第 9 节）；每类参数一行 JSON。

use forum_core::staff::ModerationSettings;
use serde_json::Value;
use sqlx::PgExecutor;

const KEY_MODERATION: &str = "moderation";

/// 管理操作限速与异常检测阈值；没有设置过时用默认值，库里的值越界时收回到上下限内。
pub async fn moderation(db: impl PgExecutor<'_>) -> Result<ModerationSettings, sqlx::Error> {
    let row = sqlx::query!(
        "SELECT value FROM site_settings WHERE key = $1",
        KEY_MODERATION
    )
    .fetch_optional(db)
    .await?;
    Ok(row
        .and_then(|r| serde_json::from_value::<ModerationSettings>(r.value).ok())
        .unwrap_or_default()
        .clamped())
}

/// 保存（调用方已校验范围）。
pub async fn set_moderation(
    db: impl PgExecutor<'_>,
    value: &ModerationSettings,
    by: i64,
) -> Result<(), sqlx::Error> {
    let json: Value = serde_json::to_value(value).map_err(|e| sqlx::Error::Encode(Box::new(e)))?;
    sqlx::query!(
        "INSERT INTO site_settings (key, value, updated_by) VALUES ($1, $2, $3)
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_by = EXCLUDED.updated_by,
                                         updated_at = now()",
        KEY_MODERATION,
        json,
        by
    )
    .execute(db)
    .await?;
    Ok(())
}
