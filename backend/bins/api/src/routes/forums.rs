//! 板块列表、板块主题列表、发主题。

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use forum_content::Cleaned;
use forum_core::ErrorCode;
use forum_core::auction::Currency;
use forum_db::topics::{self, FLAG_HIDDEN, FLAG_PINNED, Topic};
use forum_db::users::Actor;
use forum_kv::forum_cache::{self, BOARD_CAP};
use serde::Deserialize;
use serde_json::Value;
use time::OffsetDateTime;
use tower_sessions::Session;
use utoipa::{IntoParams, ToSchema};

use crate::access::{Viewer, visible_forums};
use crate::cards;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::guest::{self, Entry};
use crate::middleware::ClientIp;
use crate::posting::{self, TOPIC_RATE, after_commit, micros};
use crate::routes::auctions::AuctionState;
use crate::routes::hot::{self, TopicSort};
use crate::state::AppState;
use crate::views::{AuctionSummary, BoardResponse, CreatedTopic, ForumView, TopicSummary};

/// 每页主题数：前 200 页 × 50 = 有序集合上限 1 万。
pub const TOPICS_PER_PAGE: usize = 50;
pub const MAX_PAGE: u32 = 200;
const PINNED_TTL: Duration = Duration::from_secs(3600);

/// 访问者能看到的全部板块（平铺，前端按 `parent_id` 组树）。
#[utoipa::path(
    get, path = "/api/v1/forums", tag = "forum",
    responses((status = 200, body = Vec<ForumView>))
)]
pub async fn list(State(state): State<AppState>, session: Session) -> ApiResult<Response> {
    let viewer = Viewer::load(&state, &session).await?;
    guest::respond(&state, &viewer, Some(Entry::forums()), || async {
        let forums = visible_forums(&state, &viewer).await?;
        Ok((forums.iter().map(ForumView::from).collect::<Vec<_>>(), true))
    })
    .await
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct BoardQuery {
    /// 页码 1–200；与 `cursor` 二选一。热门不用页码。
    pub page: Option<u32>,
    /// 第 200 页之后的游标（取自上一页的 `next_cursor`）；热门翻页也用它。
    pub cursor: Option<String>,
    /// `latest`（默认，按最后回复时间，置顶帖在第 1 页）或 `hot`（热门，不含置顶帖）。
    #[param(inline)]
    pub sort: Option<TopicSort>,
}

pub fn parse_cursor(c: &str) -> ApiResult<(OffsetDateTime, i64)> {
    let bad = || ApiError(ErrorCode::InvalidParameter);
    let (us, id) = c.split_once('_').ok_or_else(bad)?;
    let us: i128 = us.parse().map_err(|_| bad())?;
    let at = OffsetDateTime::from_unix_timestamp_nanos(us * 1000).map_err(|_| bad())?;
    Ok((at, id.parse().map_err(|_| bad())?))
}

pub fn cursor_of(at_micros: i64, id: i64) -> String {
    format!("{at_micros}_{id}")
}

/// 板块主题列表（plan 第 8 节）：置顶帖单独缓存、只在第 1 页；普通帖前 200 页取自 Valkey 有序集合再按主键批量取，
/// 更深用 `(last_post_at, id)` 游标走部分索引。
#[utoipa::path(
    get, path = "/api/v1/forums/{slug}/topics", tag = "forum",
    params(("slug" = String, Path), BoardQuery),
    responses((status = 200, body = BoardResponse), (status = 400, body = ErrorBody), (status = 404, body = ErrorBody))
)]
pub async fn board(
    State(state): State<AppState>,
    session: Session,
    Path(slug): Path<String>,
    Query(q): Query<BoardQuery>,
) -> ApiResult<Response> {
    let viewer = Viewer::load(&state, &session).await?;
    let forum = forum_db::forums::by_slug(&state.db, &slug)
        .await?
        .ok_or(ApiError(ErrorCode::NotFound))?;
    if !viewer.can_view(&state, &forum).await? {
        return Err(ApiError(ErrorCode::NotFound));
    }
    let hot = q.sort == Some(TopicSort::Hot);
    if hot && q.page.is_some() {
        return Err(ApiError(ErrorCode::InvalidParameter));
    }
    // 前 200 页与热门第一页走游客缓存；游标深分页的键空间无界，不缓存（走部分索引，本来就快）
    let entry = match (hot, q.page.unwrap_or(1), &q.cursor) {
        (true, _, None) => Some(Entry::board_hot(forum.id)),
        (false, page, None) if (1..=MAX_PAGE).contains(&page) => Some(Entry::board(forum.id, page)),
        _ => None,
    };
    guest::respond(&state, &viewer, entry, || async {
        let page = if hot {
            hot::board(&state, &forum, q.cursor.as_deref()).await?
        } else {
            board_page(&state, &forum, &q).await?
        };
        Ok((page, true))
    })
    .await
}

/// 组装板块的一页（与访问者无关：可见性已在调用方检查）。
async fn board_page(
    state: &AppState,
    forum: &forum_db::forums::Forum,
    q: &BoardQuery,
) -> ApiResult<BoardResponse> {
    let mut pinned_ids = Vec::new();
    let mut next_page = None;
    let mut next_cursor = None;
    let normal: Vec<Topic>;
    if let Some(c) = q.cursor.as_deref() {
        if q.page.is_some() {
            return Err(ApiError(ErrorCode::InvalidParameter));
        }
        normal = topics::list_before(
            &state.db,
            forum.id,
            parse_cursor(c)?,
            TOPICS_PER_PAGE as i64,
        )
        .await?;
        if normal.len() == TOPICS_PER_PAGE
            && let Some(last) = normal.last()
        {
            next_cursor = Some(cursor_of(micros(last.last_post_at), last.id));
        }
    } else {
        let page = q.page.unwrap_or(1);
        if !(1..=MAX_PAGE).contains(&page) {
            return Err(ApiError(ErrorCode::InvalidParameter));
        }
        let start = (page as usize - 1) * TOPICS_PER_PAGE;
        let stop = start + TOPICS_PER_PAGE - 1;
        let board = match state.cache.board_page(forum.id, start, stop).await? {
            Some(b) => b,
            None => {
                let rows = topics::latest_listed(&state.db, forum.id, BOARD_CAP as i64).await?;
                let entries: Vec<(i64, i64)> =
                    rows.iter().map(|&(id, at)| (id, micros(at))).collect();
                state.cache.board_fill(forum.id, &entries).await?;
                state
                    .cache
                    .board_page(forum.id, start, stop)
                    .await?
                    .unwrap_or(forum_kv::BoardPage {
                        entries: Vec::new(),
                        total: 0,
                    })
            }
        };
        let full = board.entries.len() == TOPICS_PER_PAGE;
        if full && ((start + TOPICS_PER_PAGE) as u64) < board.total {
            next_page = Some(page + 1);
        } else if full
            && board.total >= BOARD_CAP as u64
            && let Some(&(id, at)) = board.entries.last()
        {
            // 有序集合到底但数据库里还有更早的主题
            next_cursor = Some(cursor_of(at, id));
        }
        if page == 1 {
            pinned_ids = pinned(state, forum.id).await?;
        }
        let ids: Vec<i64> = pinned_ids
            .iter()
            .copied()
            .chain(board.entries.iter().map(|&(id, _)| id))
            .collect();
        let mut rows: HashMap<i64, Topic> = topics::by_ids(&state.db, &ids)
            .await?
            .into_iter()
            .map(|t| (t.id, t))
            .collect();
        // 有序集合可能残留刚隐藏、置顶或移走的主题：取回后按当前状态过滤。
        normal = board
            .entries
            .iter()
            .filter_map(|(id, _)| rows.remove(id))
            .filter(|t| t.forum_id == forum.id && t.listed())
            .collect();
        let pinned_rows: Vec<Topic> = pinned_ids
            .iter()
            .filter_map(|id| rows.remove(id))
            .filter(|t| {
                t.forum_id == forum.id && t.flags & FLAG_PINNED != 0 && t.flags & FLAG_HIDDEN == 0
            })
            .collect();
        return respond(state, forum, pinned_rows, normal, next_page, next_cursor).await;
    }
    respond(state, forum, Vec::new(), normal, next_page, next_cursor).await
}

pub(crate) async fn respond(
    state: &AppState,
    forum: &forum_db::forums::Forum,
    pinned: Vec<Topic>,
    normal: Vec<Topic>,
    next_page: Option<u32>,
    next_cursor: Option<String>,
) -> ApiResult<BoardResponse> {
    let user_ids = topic_user_ids(pinned.iter().chain(&normal));
    let n = pinned.len();
    let mut all = summaries(state, &[pinned, normal].concat()).await?;
    let topics = all.split_off(n);
    Ok(BoardResponse {
        forum: forum.into(),
        pinned: all,
        topics,
        users: cards::load(state, &user_ids).await?,
        next_page,
        next_cursor,
    })
}

/// 列表行：主题摘要，交易帖补上剩余库存、拍卖帖补上当前价与结束时间（本页的交易帖、拍卖帖各一次查询）。
pub async fn summaries(state: &AppState, topics: &[Topic]) -> ApiResult<Vec<TopicSummary>> {
    let listings: Vec<i64> = topics.iter().filter_map(|t| t.listing_id).collect();
    let stock: HashMap<i64, i64> = topics::listing_stock(&state.db, &listings)
        .await?
        .into_iter()
        .collect();
    let auction_topics: Vec<i64> = topics
        .iter()
        .filter(|t| t.kind == topics::KIND_AUCTION)
        .map(|t| t.id)
        .collect();
    let now = OffsetDateTime::now_utc();
    let mut auctions: HashMap<i64, AuctionSummary> =
        forum_db::auctions::summaries(&state.db, &auction_topics)
            .await?
            .into_iter()
            .filter_map(|a| Some((a.topic_id, auction_summary(a, now)?)))
            .collect();
    Ok(topics
        .iter()
        .map(|t| {
            let mut s = TopicSummary::from(t);
            if let Some(trade) = &mut s.trade {
                trade.stock = stock.get(&trade.listing_id).copied();
            }
            s.auction = auctions.remove(&t.id);
            s
        })
        .collect())
}

fn auction_summary(a: forum_db::auctions::Summary, now: OffsetDateTime) -> Option<AuctionSummary> {
    let currency = Currency::from_code(a.currency)?;
    Some(AuctionSummary {
        auction_id: a.id,
        state: AuctionState::of(a.state, Some(a.ends_at), now)
            .as_str()
            .to_owned(),
        currency: currency.as_str().to_owned(),
        current: a.current,
        cny_per_usdt: (a.cny_per_usdt != 0).then_some(a.cny_per_usdt),
        bids: a.bid_count,
        ends_at: a.ends_at,
        reserve_met: a.reserve_met,
    })
}

/// 主题列表里出现的作者、最后回复者与参与者头像。
pub fn topic_user_ids<'a>(topics: impl Iterator<Item = &'a Topic>) -> Vec<i64> {
    topics
        .flat_map(|t| {
            [t.author_id]
                .into_iter()
                .chain(t.last_poster_id)
                .chain(t.recent_posters.iter().copied())
        })
        .collect()
}

/// 置顶帖 ID（版本化缓存，置顶 / 隐藏 / 移动时失效）。
async fn pinned(state: &AppState, forum_id: i32) -> ApiResult<Vec<i64>> {
    let name = forum_cache::pinned(forum_id);
    let (version, cached) = state.cache.versioned_get(&name).await?;
    if let Some(ids) = cached.and_then(|v| serde_json::from_str(&v).ok()) {
        return Ok(ids);
    }
    let ids = topics::pinned_ids(&state.db, forum_id).await?;
    let json = serde_json::to_string(&ids).map_err(ApiError::internal)?;
    state
        .cache
        .versioned_set(&name, &version, &json, PINNED_TTL)
        .await?;
    Ok(ids)
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewTopicRequest {
    pub title: String,
    /// Tiptap `getHTML()` 的结果；服务端会重新过滤。
    pub html: String,
    /// Tiptap `getJSON()` 原稿，重新编辑时加载。
    #[schema(value_type = Object)]
    pub source: Value,
    /// 注册 24 小时内必填：Turnstile 令牌（组件的 `action` 为 `post`）。
    pub captcha: Option<String>,
}

/// 发主题。发帖免审核（plan 第 4 节），新用户 24 小时内限频、不能发外链。
#[utoipa::path(
    post, path = "/api/v1/forums/{slug}/topics", tag = "forum",
    params(("slug" = String, Path)),
    request_body = NewTopicRequest,
    responses(
        (status = 201, body = CreatedTopic),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody), (status = 429, body = ErrorBody)
    )
)]
pub async fn create_topic(
    State(state): State<AppState>,
    session: Session,
    ClientIp(ip): ClientIp,
    Path(slug): Path<String>,
    Json(req): Json<NewTopicRequest>,
) -> ApiResult<(StatusCode, Json<CreatedTopic>)> {
    let p = prepare_topic(&state, &session, ip, &slug, &req, topics::KIND_NORMAL).await?;
    let created = topics::create(&state.db, p.new_topic(&req, topics::KIND_NORMAL)).await?;
    p.after_create(&state, &created).await;
    Ok((
        StatusCode::CREATED,
        Json(CreatedTopic {
            topic_id: created.topic_id,
            post_id: created.post_id,
        }),
    ))
}

/// 通过发帖检查（权限、过滤、限流）的新主题；交易帖、拍卖帖与普通主题共用。
pub struct PreparedTopic {
    pub actor: Actor,
    pub forum_id: i32,
    pub title: String,
    pub cleaned: Cleaned,
}

impl PreparedTopic {
    pub fn new_topic<'a>(&'a self, req: &'a NewTopicRequest, kind: i16) -> topics::NewTopic<'a> {
        topics::NewTopic {
            forum_id: self.forum_id,
            author_id: self.actor.id,
            title: &self.title,
            html: &self.cleaned.html,
            plain_text: &self.cleaned.plain_text,
            source: &req.source,
            mentions: &self.cleaned.mentions,
            kind,
        }
    }

    /// 提交后把新主题放进板块有序集合，并让游客缓存里的板块与首页失效。
    pub async fn after_create(&self, state: &AppState, created: &topics::Created) {
        after_commit(
            "board_touch",
            state.cache.board_touch(
                self.forum_id,
                created.topic_id,
                micros(created.last_post_at),
            ),
        )
        .await;
        guest::listing_changed(state, created.topic_id, &[self.forum_id]).await;
    }
}

/// 发主题前的检查：登录、板块权限（交易帖只能发在允许交易的板块，拍卖帖只能发在拍卖板块）、标题与内容过滤、新用户人机验证与限制、限流。
pub async fn prepare_topic(
    state: &AppState,
    session: &Session,
    ip: IpAddr,
    slug: &str,
    req: &NewTopicRequest,
    kind: i16,
) -> ApiResult<PreparedTopic> {
    let viewer = Viewer::load(state, session).await?;
    let actor = viewer.require()?;
    let forum = forum_db::forums::by_slug(&state.db, slug)
        .await?
        .ok_or(ApiError(ErrorCode::NotFound))?;
    if actor.level < forum.view_min_level || actor.level < forum.post_min_level {
        let staff = viewer.is_staff(state, forum.id).await?;
        if !staff && actor.level < forum.view_min_level {
            return Err(ApiError(ErrorCode::NotFound));
        }
        if !staff {
            return Err(ApiError(ErrorCode::Forbidden));
        }
    }
    posting::check_sanctions(state, actor.id, Some(forum.id)).await?;
    if kind == topics::KIND_TRADE && !forum.allow_trade {
        return Err(ApiError(ErrorCode::TradeNotAllowed));
    }
    if kind == topics::KIND_AUCTION && !forum.allow_auction {
        return Err(ApiError(ErrorCode::AuctionNotAllowed));
    }
    let title = forum_content::clean_title(&req.title)?;
    let cleaned = posting::clean(state, &actor, &req.html, &req.source)?;
    posting::check_new_user_captcha(state, &actor, req.captcha.as_deref(), ip).await?;
    posting::limit_new_content(state, &actor, &TOPIC_RATE).await?;
    Ok(PreparedTopic {
        actor,
        forum_id: forum.id,
        title,
        cleaned,
    })
}
