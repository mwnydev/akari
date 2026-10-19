//! 普通论坛等级与经验（plan 第 7 节）：只影响论坛功能和显示。经验流水只追加，等级不降。
//!
//! 同一用户的经验变动先锁住其 `user_levels` 行再记流水，每日上限的计数不会被并发请求绕过。

use sqlx::PgConnection;
use time::Date;

/// 一类经验：流水里的 `reason`、分值与每日上限（plan 第 7 节默认值；站长可调后改为读配置）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    pub reason: i16,
    pub amount: i32,
    pub daily_cap: Option<i32>,
}

/// 发主题 +2，每天计 5 帖（发出 24 小时未被删才计）。
pub const TOPIC: Rule = Rule {
    reason: 1,
    amount: 2,
    daily_cap: Some(5),
};
/// 回复 +1，每天计 20 条（回自己的主题、没有文字、重复内容不计）。
pub const REPLY: Rule = Rule {
    reason: 2,
    amount: 1,
    daily_cap: Some(20),
};
/// 被设精华 +50，撤销即扣回。
pub const FEATURED: Rule = Rule {
    reason: 3,
    amount: 50,
    daily_cap: None,
};
/// 完成交易 +10（订单 ≥ [`TRADE_MIN_USDT`]，同一对买卖双方 30 天只计 1 笔）。
pub const TRADE: Rule = Rule {
    reason: 4,
    amount: 10,
    daily_cap: None,
};
/// 争议败诉 −100。
pub const DISPUTE_LOST: Rule = Rule {
    reason: 5,
    amount: -100,
    daily_cap: None,
};

/// 完成交易计经验的最低订单金额（整 USDT）。
pub const TRADE_MIN_USDT: i32 = 20;

/// 一次经验发放的来源。
#[derive(Debug, Clone, Copy)]
pub struct Grant<'a> {
    pub user_id: i64,
    /// 帖子 / 主题 / 订单 ID。
    pub ref_id: Option<i64>,
    /// 来源事件的 UTC 日期（每日上限按它计）。
    pub day: Date,
    /// 同一来源只计一次。
    pub dedupe: Option<&'a str>,
}

/// 锁住用户的等级行（没有就先建），返回当前等级。
async fn lock(conn: &mut PgConnection, user_id: i64) -> Result<i16, sqlx::Error> {
    sqlx::query!(
        "INSERT INTO user_levels (user_id) VALUES ($1) ON CONFLICT (user_id) DO NOTHING",
        user_id
    )
    .execute(&mut *conn)
    .await?;
    let row = sqlx::query_file!("queries/exp_lock.sql", user_id)
        .fetch_one(&mut *conn)
        .await?;
    Ok(row.level)
}

/// 已锁住用户（`level` 为锁住时的等级）后记流水并累加经验；没记（重复来源或当天已满）返回 `None`，
/// 否则返回等级是否变化。
async fn record(
    conn: &mut PgConnection,
    level: i16,
    rule: &Rule,
    amount: i32,
    g: Grant<'_>,
) -> Result<Option<bool>, sqlx::Error> {
    let inserted = sqlx::query_file!(
        "queries/exp_insert.sql",
        g.user_id,
        rule.reason,
        amount,
        g.ref_id,
        g.day,
        g.dedupe,
        rule.daily_cap
    )
    .fetch_optional(&mut *conn)
    .await?;
    if inserted.is_none() {
        return Ok(None);
    }
    let after = sqlx::query_file!("queries/exp_apply.sql", g.user_id, i64::from(amount))
        .fetch_one(&mut *conn)
        .await?;
    Ok(Some(after.level != level))
}

/// 按规则发放一次经验。没发（重复来源或当天已满）返回 `None`，否则返回等级是否变化。
pub async fn award(
    conn: &mut PgConnection,
    rule: &Rule,
    g: Grant<'_>,
) -> Result<Option<bool>, sqlx::Error> {
    let level = lock(conn, g.user_id).await?;
    record(conn, level, rule, rule.amount, g).await
}

/// 设精华加分、撤销扣回：同一主题累计给作者的精华经验只会是 0 或 +50。返回值同 [`award`]。
pub async fn set_featured(
    conn: &mut PgConnection,
    author_id: i64,
    topic_id: i64,
    featured: bool,
    day: Date,
) -> Result<Option<bool>, sqlx::Error> {
    let level = lock(conn, author_id).await?;
    let net = sqlx::query_file!("queries/exp_net.sql", author_id, FEATURED.reason, topic_id)
        .fetch_one(&mut *conn)
        .await?
        .net;
    let target = if featured { FEATURED.amount } else { 0 };
    let delta = i64::from(target) - net;
    let Ok(delta) = i32::try_from(delta) else {
        return Ok(None);
    };
    if delta == 0 {
        return Ok(None);
    }
    let g = Grant {
        user_id: author_id,
        ref_id: Some(topic_id),
        day,
        dedupe: None,
    };
    record(conn, level, &FEATURED, delta, g).await
}

/// 等待发放发帖经验的帖子（发出满 24 小时）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuePost {
    pub id: i64,
    pub author_id: i64,
    pub floor: i32,
    pub status: i16,
    pub plain_text: String,
    pub created_at: time::OffsetDateTime,
    pub topic_author_id: i64,
    pub topic_flags: i16,
}

/// 主键在 `after_id` 之后、发出满 24 小时的帖子，按主键升序。
pub async fn posts_due(
    conn: &mut PgConnection,
    after_id: i64,
    limit: i64,
) -> Result<Vec<DuePost>, sqlx::Error> {
    sqlx::query_file_as!(DuePost, "queries/exp_posts_due.sql", after_id, limit)
        .fetch_all(conn)
        .await
}
