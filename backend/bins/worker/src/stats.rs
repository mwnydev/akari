//! 每日从 `escrow_orders` 全量重算账号交易信誉（plan 第 7、8 节「每日从链上全量重算校对」）。
//! indexer 已在订单最终确认时增量更新；这里重算出不一致的行即为偏差，记录告警并以重算结果修正。

use forum_db::{PgPool, reputation};
use forum_kv::ForumCache;

/// 重算间隔。
pub const INTERVAL: std::time::Duration = std::time::Duration::from_secs(86_400);
const BATCH: i64 = 500;

/// 返回修正的用户数。
pub async fn audit_trade_stats(db: &PgPool, cache: &ForumCache) -> Result<usize, sqlx::Error> {
    let mut after = 0;
    let mut repaired = 0;
    loop {
        let ids = reputation::user_ids_after(db, after, BATCH).await?;
        let Some(&last) = ids.last() else {
            return Ok(repaired);
        };
        let drift = reputation::refresh_stats(db, &ids).await?;
        if !drift.is_empty() {
            tracing::warn!(
                users = drift.len(),
                sample = ?&drift[..drift.len().min(10)],
                "trade stats drift repaired"
            );
            if let Err(e) = cache.cards_invalidate(&drift).await {
                tracing::warn!(error = %e, "user card invalidation failed");
            }
            repaired += drift.len();
        }
        after = last;
    }
}
