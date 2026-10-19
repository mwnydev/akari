//! 热门排序（`sort=hot`，板块与首页）：排名由 forum-worker 每 5 分钟算进 `hot_topics`（[`forum_db::hot`]），
//! 这里按分数从高到低读，用 `(分数, 主题 ID)` 游标翻页。排名重算后游标仍然有效（从新排名里比它低的继续），
//! 只是相邻两页之间可能有主题换了位置。

use std::collections::HashMap;

use forum_core::ErrorCode;
use forum_db::forums::Forum;
use forum_db::hot::{self, Ranked};
use forum_db::topics::{self, Topic};
use serde::Deserialize;
use utoipa::ToSchema;

use super::forums::{TOPICS_PER_PAGE, respond};
use super::topics::latest_response;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::views::{BoardResponse, LatestResponse};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TopicSort {
    /// 按最后回复时间（默认）。
    Latest,
    /// 热门：回复与 1 楼点赞按发帖时间衰减（排名每 5 分钟更新）。
    Hot,
}

/// 游标 `分数_主题ID`（分数是 f64 的最短往返写法）。
fn parse_cursor(c: &str) -> ApiResult<(f64, i64)> {
    let bad = || ApiError(ErrorCode::InvalidParameter);
    let (score, id) = c.split_once('_').ok_or_else(bad)?;
    let score: f64 = score.parse().map_err(|_| bad())?;
    if !score.is_finite() {
        return Err(bad());
    }
    Ok((score, id.parse().map_err(|_| bad())?))
}

fn cursor_of(r: Ranked) -> String {
    format!("{}_{}", r.score, r.topic_id)
}

/// 按排名取主题，按当前状态过滤（排名里可能有刚隐藏、置顶或移走的主题），保持排名顺序。
/// 排名取满一页时给出下一页游标（过滤掉一些时本页会少于一页）。
async fn ranked_topics(
    state: &AppState,
    ranked: &[Ranked],
    keep: impl Fn(&Topic) -> bool,
) -> ApiResult<(Vec<Topic>, Option<String>)> {
    let ids: Vec<i64> = ranked.iter().map(|r| r.topic_id).collect();
    let mut rows: HashMap<i64, Topic> = topics::by_ids(&state.db, &ids)
        .await?
        .into_iter()
        .map(|t| (t.id, t))
        .collect();
    let out = ranked
        .iter()
        .filter_map(|r| rows.remove(&r.topic_id))
        .filter(|t| t.listed() && keep(t))
        .collect();
    let next = ranked
        .last()
        .filter(|_| ranked.len() == TOPICS_PER_PAGE)
        .map(|r| cursor_of(*r));
    Ok((out, next))
}

/// 板块热门（不含置顶帖）。
pub async fn board(
    state: &AppState,
    forum: &Forum,
    cursor: Option<&str>,
) -> ApiResult<BoardResponse> {
    let cursor = cursor.map(parse_cursor).transpose()?;
    let ranked = hot::board(&state.db, forum.id, cursor, TOPICS_PER_PAGE as i64).await?;
    let (topics, next) = ranked_topics(state, &ranked, |t| t.forum_id == forum.id).await?;
    respond(state, forum, Vec::new(), topics, None, next).await
}

/// 首页热门：只含访问者看得到的板块。
pub async fn latest(
    state: &AppState,
    forums: &HashMap<i32, Forum>,
    cursor: Option<&str>,
) -> ApiResult<LatestResponse> {
    let cursor = cursor.map(parse_cursor).transpose()?;
    let ids: Vec<i32> = forums.keys().copied().collect();
    let ranked = hot::site(&state.db, &ids, cursor, TOPICS_PER_PAGE as i64).await?;
    let (topics, next) =
        ranked_topics(state, &ranked, |t| forums.contains_key(&t.forum_id)).await?;
    latest_response(state, forums, &topics, next).await
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn cursor_round_trips_exactly() {
        for score in [0.0, 1.0 / 3.0, 12_345.678_901_234_5, f64::MIN_POSITIVE] {
            let r = Ranked {
                topic_id: 42,
                score,
            };
            let (s, id) = parse_cursor(&cursor_of(r)).unwrap();
            assert_eq!((s.to_bits(), id), (score.to_bits(), 42));
        }
        for bad in ["", "1.5", "x_1", "1.5_x", "inf_1", "NaN_2"] {
            assert!(parse_cursor(bad).is_err(), "{bad}");
        }
    }
}
