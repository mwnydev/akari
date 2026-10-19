//! 账号交易信誉与交易经验（plan 第 7 节）。只认最终确认的订单结果（plan 第 8 节「区块回滚」）：
//! indexer 在订单变为最终确认的同一事务里调用 [`apply_finalized`]，worker 每日用 [`refresh_stats`] 全量重算校对。
//! 订单结果的分类只在 SQL 函数 `escrow_outcome` 里定义一次。

use sqlx::{PgConnection, PgExecutor};
use time::Date;

use crate::exp::{self, Grant};

/// 计入信誉的最低订单金额（整 USDT）。
pub const STATS_MIN_USDT: i32 = 10;

/// 订单结果（`escrow_outcome` 的取值）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// 确认放款、确认期满放款、分期到期：双方成功。
    Success,
    /// 卖家未按期发货（或发货前退款）：卖家失败。
    SellerNoShow,
    /// 仲裁判卖家责任：卖家失败 + 争议败诉。
    SellerLost,
    /// 仲裁判买家责任：卖家成功，买家失败 + 争议败诉。
    BuyerLost,
    /// 和解等不计入成功率的结果。
    Settled,
}

impl Outcome {
    pub fn from_code(code: i16) -> Option<Self> {
        Some(match code {
            1 => Self::Success,
            2 => Self::SellerNoShow,
            3 => Self::SellerLost,
            4 => Self::BuyerLost,
            5 => Self::Settled,
            _ => return None,
        })
    }
}

/// 从这些用户最终确认的订单重算信誉并写入，返回实际变化的用户。
pub async fn refresh_stats(
    db: impl PgExecutor<'_>,
    user_ids: &[i64],
) -> Result<Vec<i64>, sqlx::Error> {
    if user_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query_file!(
        "queries/user_trade_stats_refresh.sql",
        user_ids,
        STATS_MIN_USDT
    )
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(|r| r.user_id).collect())
}

/// 一张最终确认、已结束订单的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinalOrder {
    pub id: i64,
    pub buyer_id: Option<i64>,
    pub seller_id: Option<i64>,
    pub outcome: Outcome,
    /// 金额达到 [`exp::TRADE_MIN_USDT`]。
    pub qualifies: bool,
    /// 建单时间所在的 30 天窗口。
    pub window: i64,
    pub day: Date,
}

pub async fn final_orders(
    db: impl PgExecutor<'_>,
    order_ids: &[i64],
) -> Result<Vec<FinalOrder>, sqlx::Error> {
    if order_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query_file!(
        "queries/trade_outcomes_by_ids.sql",
        order_ids,
        exp::TRADE_MIN_USDT
    )
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            Some(FinalOrder {
                id: r.id,
                buyer_id: r.buyer_id,
                seller_id: r.seller_id,
                outcome: Outcome::from_code(r.outcome)?,
                qualifies: r.qualifies,
                window: r.window,
                day: r.day,
            })
        })
        .collect())
}

/// 订单变为最终确认后：发交易经验、更新双方信誉。不是最终确认或未结束的订单忽略；重复调用不会重复发放。
/// 返回名片需要失效的用户（信誉或等级变化）。
pub async fn apply_finalized(
    conn: &mut PgConnection,
    order_ids: &[i64],
) -> Result<Vec<i64>, sqlx::Error> {
    let orders = final_orders(&mut *conn, order_ids).await?;
    let mut users = Vec::new();
    let mut changed = Vec::new();
    for o in &orders {
        let (Some(buyer), Some(seller)) = (o.buyer_id, o.seller_id) else {
            continue;
        };
        if buyer == seller {
            continue;
        }
        users.extend([buyer, seller]);
        let order_key = format!("order:{}", o.id);
        let pair_key = format!(
            "pair:{}:{}:{}",
            buyer.min(seller),
            buyer.max(seller),
            o.window
        );
        let grants: Vec<(i64, &exp::Rule, &str)> = match o.outcome {
            Outcome::Success if o.qualifies => {
                vec![
                    (buyer, &exp::TRADE, &pair_key),
                    (seller, &exp::TRADE, &pair_key),
                ]
            }
            Outcome::SellerLost => vec![(seller, &exp::DISPUTE_LOST, &order_key)],
            Outcome::BuyerLost => vec![(buyer, &exp::DISPUTE_LOST, &order_key)],
            _ => Vec::new(),
        };
        for (user_id, rule, key) in grants {
            let g = Grant {
                user_id,
                ref_id: Some(o.id),
                day: o.day,
                dedupe: Some(key),
            };
            if exp::award(conn, rule, g).await? == Some(true) {
                changed.push(user_id);
            }
        }
    }
    users.sort_unstable();
    users.dedup();
    changed.extend(refresh_stats(&mut *conn, &users).await?);
    changed.sort_unstable();
    changed.dedup();
    Ok(changed)
}

/// 每日全量重算按用户 ID 分批：`after` 之后的 `limit` 个用户。
pub async fn user_ids_after(
    db: impl PgExecutor<'_>,
    after: i64,
    limit: i64,
) -> Result<Vec<i64>, sqlx::Error> {
    let rows = sqlx::query!(
        "SELECT id FROM users WHERE id > $1 ORDER BY id LIMIT $2",
        after,
        limit
    )
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(|r| r.id).collect())
}
