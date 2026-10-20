//! 用户名片（plan 第 8 节「用户名片：Valkey 哈希缓存，未命中查主键」）：整页一次批量取，先读缓存，
//! 未命中的一次查库再写回。缓存读写失败时直接查库（数据库是唯一事实来源）。

use std::collections::HashMap;

use forum_db::users::{AuctionRecord, Card, TradeCounts};
use time::OffsetDateTime;

use crate::error::ApiResult;
use crate::state::AppState;
use crate::views::UserCardView;

/// 名片哈希的字段。用户名、头像、违约禁拍结束时间为空时不写该字段。
fn fields(c: &Card) -> Vec<(&'static str, String)> {
    let t = &c.trade;
    let mut f = vec![
        ("l", c.level.to_string()),
        ("s", t.success.to_string()),
        ("sf", t.seller_fault_fail.to_string()),
        ("bf", t.buyer_fault_fail.to_string()),
        ("d", t.disputes_lost.to_string()),
        ("st", t.settled.to_string()),
        ("ad", c.auction.defaults.to_string()),
    ];
    if let Some(u) = &c.username {
        f.push(("u", u.clone()));
    }
    if let Some(a) = &c.avatar_url {
        f.push(("a", a.clone()));
    }
    if let Some(b) = c.auction.banned_until {
        f.push(("bu", b.unix_timestamp().to_string()));
    }
    f
}

/// 从缓存哈希还原；缺字段或格式不对（旧版本写入的）当作未命中。
fn from_fields(id: i64, m: &HashMap<String, String>) -> Option<Card> {
    let num = |k: &str| m.get(k)?.parse::<i32>().ok();
    Some(Card {
        id,
        username: m.get("u").cloned(),
        avatar_url: m.get("a").cloned(),
        level: m.get("l")?.parse().ok()?,
        trade: TradeCounts {
            success: num("s")?,
            seller_fault_fail: num("sf")?,
            buyer_fault_fail: num("bf")?,
            disputes_lost: num("d")?,
            settled: num("st")?,
        },
        auction: AuctionRecord {
            defaults: num("ad")?,
            banned_until: match m.get("bu") {
                Some(v) => Some(OffsetDateTime::from_unix_timestamp(v.parse().ok()?).ok()?),
                None => None,
            },
        },
    })
}

/// 批量取名片（`ids` 去重后按原顺序返回存在的用户）。
pub async fn load(state: &AppState, ids: &[i64]) -> ApiResult<Vec<UserCardView>> {
    let mut seen = std::collections::HashSet::new();
    let ids: Vec<i64> = ids.iter().copied().filter(|id| seen.insert(*id)).collect();
    let cached = match state.cache.cards_get(&ids).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "card cache read failed");
            vec![None; ids.len()]
        }
    };
    let mut found: HashMap<i64, Card> = HashMap::new();
    let mut missing = Vec::new();
    for (&id, entry) in ids.iter().zip(&cached) {
        match entry.as_ref().and_then(|m| from_fields(id, m)) {
            Some(card) => {
                found.insert(id, card);
            }
            None => missing.push(id),
        }
    }
    if !missing.is_empty() {
        let rows = forum_db::users::cards(&state.db, &missing).await?;
        let fill: Vec<(i64, Vec<(&str, String)>)> =
            rows.iter().map(|c| (c.id, fields(c))).collect();
        if let Err(e) = state.cache.cards_put(&fill).await {
            tracing::warn!(error = %e, "card cache write failed");
        }
        found.extend(rows.into_iter().map(|c| (c.id, c)));
    }
    Ok(ids
        .iter()
        .filter_map(|id| found.remove(id))
        .map(UserCardView::from)
        .collect())
}

/// 用户名、头像、等级、信誉或拍卖违约记录（含违约禁拍被解除）变化后失效。
pub async fn invalidate(state: &AppState, ids: &[i64]) {
    crate::posting::after_commit("cards_invalidate", state.cache.cards_invalidate(ids)).await;
}
