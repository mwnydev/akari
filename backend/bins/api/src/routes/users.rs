//! 用户：个人主页、用户的帖子、@提及搜索、名片批量取。

use std::time::Duration;

use axum::Json;
use axum::extract::{Path, Query, State};
use forum_core::ErrorCode;
use forum_kv::forum_cache;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tower_sessions::Session;
use utoipa::{IntoParams, ToSchema};

use crate::access::{Viewer, visible_forums};
use crate::cards;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::state::AppState;
use crate::views::{ReputationView, UserCardView};

/// 主页帖子每页条数。
const POSTS_PER_PAGE: i64 = 20;
/// @提及候选最多条数。
const MENTION_LIMIT: i64 = 10;
/// 名片批量接口一次最多取多少个。
const MAX_CARDS: usize = 100;
/// 主题数、帖子数的缓存时间（个人主页不要求实时）。
const COUNTS_TTL: Duration = Duration::from_secs(300);

/// 路径里的用户：纯数字按 ID，否则按用户名（用户名不能是纯数字）。
async fn resolve(state: &AppState, key: &str) -> ApiResult<i64> {
    let id = if !key.is_empty() && key.bytes().all(|b| b.is_ascii_digit()) {
        key.parse().ok()
    } else {
        forum_db::users::id_by_name(&state.db, key).await?
    };
    id.ok_or(ApiError(ErrorCode::NotFound))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TradeStatsView {
    #[serde(flatten)]
    pub summary: ReputationView,
    /// 作为卖家的责任失败。
    pub seller_fault_fail: i32,
    /// 作为买家的责任失败。
    pub buyer_fault_fail: i32,
    /// 成功订单金额合计（USDT 最小单位，十进制字符串）。
    pub volume: String,
    /// 卖家平均回复用时（秒）；私信功能上线后才有。
    pub avg_reply_secs: Option<i32>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProfileView {
    pub id: i64,
    pub username: Option<String>,
    pub avatar_url: Option<String>,
    pub level: i16,
    pub exp: i64,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub joined_at: OffsetDateTime,
    /// 最近活跃时间。用户隐身时只精确到天（当天 0 点，UTC），`last_active_day_only` 为真。
    #[schema(value_type = Option<String>, format = DateTime)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_active_at: Option<OffsetDateTime>,
    pub last_active_day_only: bool,
    /// 未删除的主题数与帖子数（含主题 1 楼），最多 5 分钟延迟。
    pub topics: i64,
    pub posts: i64,
    pub trade: TradeStatsView,
}

/// 个人主页。`{user}` 为用户 ID 或用户名（不区分大小写）。
#[utoipa::path(
    get, path = "/api/v1/users/{user}", tag = "users",
    params(("user" = String, Path, description = "用户 ID 或用户名")),
    responses((status = 200, body = ProfileView), (status = 404, body = ErrorBody))
)]
pub async fn user_profile(
    State(state): State<AppState>,
    Path(user): Path<String>,
) -> ApiResult<Json<ProfileView>> {
    let id = resolve(&state, &user).await?;
    let p = forum_db::users::profile(&state.db, id)
        .await?
        .ok_or(ApiError(ErrorCode::NotFound))?;
    let (topics, posts) = counts(&state, id).await?;
    let (last_active_at, day_only) = match p.last_seen_at {
        Some(t) if !p.show_online => (Some(t.replace_time(time::Time::MIDNIGHT)), true),
        t => (t, false),
    };
    Ok(Json(ProfileView {
        id: p.id,
        username: p.username,
        avatar_url: p.avatar_url,
        level: p.level,
        exp: p.exp,
        joined_at: p.created_at,
        last_active_at,
        last_active_day_only: day_only,
        topics,
        posts,
        trade: TradeStatsView {
            summary: p.trade.into(),
            seller_fault_fail: p.trade.seller_fault_fail,
            buyer_fault_fail: p.trade.buyer_fault_fail,
            volume: p.volume,
            avg_reply_secs: p.avg_reply_secs,
        },
    }))
}

/// 主题数与帖子数，缓存 5 分钟（发帖多的用户计数要扫很多索引项）。
async fn counts(state: &AppState, id: i64) -> ApiResult<(i64, i64)> {
    let name = forum_cache::user_counts(id);
    let (version, cached) = state.cache.versioned_get(&name).await?;
    if let Some(c) = cached.and_then(|v| serde_json::from_str(&v).ok()) {
        return Ok(c);
    }
    let c = forum_db::users::post_counts(&state.db, id).await?;
    let json = serde_json::to_string(&c).map_err(ApiError::internal)?;
    state
        .cache
        .versioned_set(&name, &version, &json, COUNTS_TTL)
        .await?;
    Ok(c)
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct PostsQuery {
    /// 上一页的 `next_cursor`。
    pub cursor: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserPostView {
    pub id: i64,
    pub topic_id: i64,
    pub topic_title: String,
    pub forum_id: i32,
    pub floor: i32,
    /// 楼层所在页码（每页 20 楼）。
    pub page: i32,
    /// 纯文本前 200 字。
    pub excerpt: String,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserPosts {
    pub posts: Vec<UserPostView>,
    pub next_cursor: Option<i64>,
}

/// 用户发过的帖子，新的在前。只含访问者看得到的板块里、未删除、主题未隐藏的楼层。
#[utoipa::path(
    get, path = "/api/v1/users/{user}/posts", tag = "users",
    params(("user" = String, Path, description = "用户 ID 或用户名"), PostsQuery),
    responses((status = 200, body = UserPosts), (status = 404, body = ErrorBody))
)]
pub async fn user_posts(
    State(state): State<AppState>,
    session: Session,
    Path(user): Path<String>,
    Query(q): Query<PostsQuery>,
) -> ApiResult<Json<UserPosts>> {
    let id = resolve(&state, &user).await?;
    let viewer = Viewer::load(&state, &session).await?;
    let forums: Vec<i32> = visible_forums(&state, &viewer)
        .await?
        .iter()
        .map(|f| f.id)
        .collect();
    let rows = forum_db::users::posts(
        &state.db,
        id,
        q.cursor.unwrap_or(i64::MAX),
        &forums,
        POSTS_PER_PAGE,
    )
    .await?;
    let next_cursor = (rows.len() as i64 == POSTS_PER_PAGE)
        .then(|| rows.last().map(|p| p.id))
        .flatten();
    Ok(Json(UserPosts {
        posts: rows
            .into_iter()
            .map(|p| UserPostView {
                page: (p.floor - 1) / forum_db::posts::PAGE_SIZE + 1,
                id: p.id,
                topic_id: p.topic_id,
                topic_title: p.title,
                forum_id: p.forum_id,
                floor: p.floor,
                excerpt: p.excerpt,
                created_at: p.created_at,
            })
            .collect(),
        next_cursor,
    }))
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct SearchQuery {
    /// 用户名前缀（不区分大小写）。
    pub q: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MentionView {
    pub id: i64,
    pub username: String,
    pub avatar_url: Option<String>,
    pub level: i16,
}

/// @提及候选：用户名以 `q` 开头的用户，最多 10 个，按用户名排序。`q` 含用户名不允许的字符时返回空列表。
#[utoipa::path(
    get, path = "/api/v1/users/search", tag = "users",
    params(SearchQuery),
    responses((status = 200, body = Vec<MentionView>))
)]
pub async fn search_users(
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
) -> ApiResult<Json<Vec<MentionView>>> {
    Ok(Json(search_prefix(&state, &q.q).await?))
}

/// 用户名以 `prefix` 开头的用户（论坛 @提及与后台任命共用）；含用户名不允许的字符时为空。
pub(crate) async fn search_prefix(state: &AppState, prefix: &str) -> ApiResult<Vec<MentionView>> {
    let prefix = prefix.trim();
    let n = prefix.chars().count();
    if n == 0
        || n > forum_core::username::MAX_CHARS
        || !prefix.chars().all(forum_core::username::is_name_char)
    {
        return Ok(Vec::new());
    }
    let rows = forum_db::users::search_prefix(&state.db, prefix, MENTION_LIMIT).await?;
    Ok(rows
        .into_iter()
        .map(|m| MentionView {
            id: m.id,
            username: m.username,
            avatar_url: m.avatar_url,
            level: m.level,
        })
        .collect())
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct CardsQuery {
    /// 逗号分隔的用户 ID，最多 100 个。
    pub ids: String,
}

/// 名片批量取（Valkey 缓存，未命中一次查库）。不存在的 ID 不出现在结果里。
#[utoipa::path(
    get, path = "/api/v1/users/cards", tag = "users",
    params(CardsQuery),
    responses((status = 200, body = Vec<UserCardView>), (status = 400, body = ErrorBody))
)]
pub async fn user_cards(
    State(state): State<AppState>,
    Query(q): Query<CardsQuery>,
) -> ApiResult<Json<Vec<UserCardView>>> {
    let ids = q
        .ids
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| s.trim().parse::<i64>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ApiError(ErrorCode::InvalidParameter))?;
    if ids.len() > MAX_CARDS {
        return Err(ApiError(ErrorCode::InvalidParameter));
    }
    Ok(Json(cards::load(&state, &ids).await?))
}
