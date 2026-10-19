//! 热门排序：forum-worker 定时把排名算进 `hot_topics`（迁移 20261211000001 说明了为什么不存成 topics 的列），
//! 板块与首页的「热门」按分数从高到低读这张表。

use sqlx::{PgExecutor, PgPool};

/// 每个板块保留的主题数（板块热门最多翻到这里）。
pub const PER_FORUM: i64 = 1000;

/// 重算排名：整表在一个事务里替换，读的人总是看到完整的上一版。返回排名里的主题数。
/// 多个 worker 同时重算时只有拿到锁的那个做，其余直接返回 0。
pub async fn refresh(db: &PgPool) -> Result<usize, sqlx::Error> {
    let mut tx = db.begin().await?;
    let locked = sqlx::query_scalar!(
        r#"SELECT pg_try_advisory_xact_lock(hashtextextended('hot-topics', 0)) AS "locked!""#
    )
    .fetch_one(&mut *tx)
    .await?;
    if !locked {
        return Ok(0);
    }
    sqlx::query!("DELETE FROM hot_topics")
        .execute(&mut *tx)
        .await?;
    let n = sqlx::query_file!("queries/hot_refresh.sql", PER_FORUM)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    tx.commit().await?;
    Ok(usize::try_from(n).unwrap_or(usize::MAX))
}

/// 排名里的一个主题。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ranked {
    pub topic_id: i64,
    pub score: f64,
}

/// 游标：上一页最后一个的 `(分数, 主题 ID)`；第一页为空。
fn after(cursor: Option<(f64, i64)>) -> (f64, i64) {
    cursor.unwrap_or((f64::INFINITY, i64::MAX))
}

/// 板块热门，分数从高到低。
pub async fn board(
    db: impl PgExecutor<'_>,
    forum_id: i32,
    cursor: Option<(f64, i64)>,
    limit: i64,
) -> Result<Vec<Ranked>, sqlx::Error> {
    let (score, id) = after(cursor);
    sqlx::query_file_as!(Ranked, "queries/hot_board.sql", forum_id, score, id, limit)
        .fetch_all(db)
        .await
}

/// 首页热门：只取 `forum_ids` 里的板块。
pub async fn site(
    db: impl PgExecutor<'_>,
    forum_ids: &[i32],
    cursor: Option<(f64, i64)>,
    limit: i64,
) -> Result<Vec<Ranked>, sqlx::Error> {
    let (score, id) = after(cursor);
    sqlx::query_file_as!(
        Ranked,
        "queries/hot_latest.sql",
        forum_ids,
        score,
        id,
        limit
    )
    .fetch_all(db)
    .await
}
