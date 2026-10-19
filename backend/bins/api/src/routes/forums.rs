//! 板块列表、板块主题列表、发主题。

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use forum_content::Cleaned;
use forum_core::ErrorCode;
use forum_db::topics::{self, FLAG_HIDDEN, FLAG_PINNED, Topic};
use forum_db::users::Actor;
use forum_kv::forum_cache::{self, BOARD_CAP};
use serde::Deserialize;
use serde_json::Value;
use time::OffsetDateTime;
use tower_sessions::Session;
use utoipa::{IntoParams, ToSchema};

use crate::access::Viewer;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::posting::{self, TOPIC_RATE, after_commit, micros};
use crate::state::AppState;
use crate::views::{BoardResponse, CreatedTopic, ForumView, TopicSummary, UserCardView};

/// 每页主题数：前 200 页 × 50 = 有序集合上限 1 万。
pub const TOPICS_PER_PAGE: usize = 50;
pub const MAX_PAGE: u32 = 200;
const PINNED_TTL: Duration = Duration::from_secs(3600);

/// 访问者能看到的全部板块（平铺，前端按 `parent_id` 组树）。
#[utoipa::path(
    get, path = "/api/v1/forums", tag = "forum",
    responses((status = 200, body = Vec<ForumView>))
)]
pub async fn list(
    State(state): State<AppState>,
    session: Session,
) -> ApiResult<Json<Vec<ForumView>>> {
    let viewer = Viewer::load(&state, &session).await?;
    let all = forum_db::forums::list_all(&state.db).await?;
    let level = viewer.level();
    // 等级不够的板块只有管理人员看得到；一次查出管理范围，不逐个板块查。
    let (global, moderated) = match viewer.user {
        Some(u) if all.iter().any(|f| f.view_min_level > level) => {
            forum_db::staff::scope(&state.db, u.id).await?
        }
        _ => (false, Vec::new()),
    };
    Ok(Json(
        all.iter()
            .filter(|f| f.view_min_level <= level || global || moderated.contains(&f.id))
            .map(ForumView::from)
            .collect(),
    ))
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct BoardQuery {
    /// 页码 1–200；与 `cursor` 二选一。
    pub page: Option<u32>,
    /// 第 200 页之后的游标（取自上一页的 `next_cursor`）。
    pub cursor: Option<String>,
}

fn parse_cursor(c: &str) -> ApiResult<(OffsetDateTime, i64)> {
    let bad = || ApiError(ErrorCode::InvalidParameter);
    let (us, id) = c.split_once('_').ok_or_else(bad)?;
    let us: i128 = us.parse().map_err(|_| bad())?;
    let at = OffsetDateTime::from_unix_timestamp_nanos(us * 1000).map_err(|_| bad())?;
    Ok((at, id.parse().map_err(|_| bad())?))
}

fn cursor_of(at_micros: i64, id: i64) -> String {
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
) -> ApiResult<Json<BoardResponse>> {
    let viewer = Viewer::load(&state, &session).await?;
    let forum = forum_db::forums::by_slug(&state.db, &slug)
        .await?
        .ok_or(ApiError(ErrorCode::NotFound))?;
    if !viewer.can_view(&state, &forum).await? {
        return Err(ApiError(ErrorCode::NotFound));
    }

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
            pinned_ids = pinned(&state, forum.id).await?;
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
        return respond(&state, &forum, pinned_rows, normal, next_page, next_cursor).await;
    }
    respond(&state, &forum, Vec::new(), normal, next_page, next_cursor).await
}

async fn respond(
    state: &AppState,
    forum: &forum_db::forums::Forum,
    pinned: Vec<Topic>,
    normal: Vec<Topic>,
    next_page: Option<u32>,
    next_cursor: Option<String>,
) -> ApiResult<Json<BoardResponse>> {
    let user_ids: Vec<i64> = pinned
        .iter()
        .chain(&normal)
        .flat_map(|t| [Some(t.author_id), t.last_poster_id])
        .flatten()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let users = forum_db::users::cards(&state.db, &user_ids).await?;
    Ok(Json(BoardResponse {
        forum: forum.into(),
        pinned: pinned.iter().map(TopicSummary::from).collect(),
        topics: normal.iter().map(TopicSummary::from).collect(),
        users: users.into_iter().map(UserCardView::from).collect(),
        next_page,
        next_cursor,
    }))
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
    Path(slug): Path<String>,
    Json(req): Json<NewTopicRequest>,
) -> ApiResult<(StatusCode, Json<CreatedTopic>)> {
    let p = prepare_topic(&state, &session, &slug, &req).await?;
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

/// 通过发帖检查（权限、过滤、限流）的新主题；交易帖与普通主题共用。
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

    /// 提交后把新主题放进板块有序集合。
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
    }
}

/// 发主题前的检查：登录、板块权限、标题与内容过滤、新用户限制与限流。
pub async fn prepare_topic(
    state: &AppState,
    session: &Session,
    slug: &str,
    req: &NewTopicRequest,
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
    let title = forum_content::clean_title(&req.title)?;
    let cleaned = posting::clean(state, &actor, &req.html, &req.source)?;
    posting::limit_new_content(state, &actor, &TOPIC_RATE).await?;
    Ok(PreparedTopic {
        actor,
        forum_id: forum.id,
        title,
        cleaned,
    })
}
