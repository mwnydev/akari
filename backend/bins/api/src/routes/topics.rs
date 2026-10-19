//! 主题页、回帖、主题管理（置顶 / 精华 / 锁定 / 隐藏 / 移动）。

use std::collections::HashMap;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use forum_core::ErrorCode;
use forum_core::staff::{Permission, Scope};
use forum_db::forums::Forum;
use forum_db::posts::{self, PAGE_SIZE};
use forum_db::topics::Topic;
use forum_db::topics::{self, FLAG_FEATURED, FLAG_HIDDEN, FLAG_LOCKED, FLAG_PINNED};
use forum_kv::BoardPage;
use forum_kv::forum_cache::{self, BOARD_CAP};
use serde::Deserialize;
use serde_json::Value;
use time::OffsetDateTime;
use tower_sessions::Session;
use utoipa::{IntoParams, ToSchema};

use crate::access::{Viewer, visible_forums};
use crate::cards;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::moderation::{self, Class, Moderator, TopicChange};
use crate::posting::{self, REPLY_RATE, after_commit, micros};
use crate::routes::forums::{TOPICS_PER_PAGE, cursor_of, parse_cursor, topic_user_ids};
use crate::state::AppState;
use crate::views::{
    CreatedPost, ForumView, LatestResponse, ModerationResult, PostView, TopicPage, TopicPageBody,
    TopicSummary,
};

/// 主题第 1 页缓存时长；内容变化时立即失效，这只是兜底。
const PAGE1_TTL: Duration = Duration::from_secs(600);

#[derive(Debug, Deserialize, IntoParams)]
pub struct TopicQuery {
    /// 页码，从 1 开始；每页 20 楼。
    pub page: Option<i32>,
}

/// 组装主题某一页（与访问者无关）。主题不存在返回 `None`。
async fn build_body(
    state: &AppState,
    topic_id: i64,
    page: i32,
) -> ApiResult<Option<TopicPageBody>> {
    let Some(topic) = topics::by_id(&state.db, topic_id).await? else {
        return Ok(None);
    };
    let Some(forum) = forum_db::forums::by_id(&state.db, topic.forum_id).await? else {
        return Ok(None);
    };
    let rows = posts::page(&state.db, topic_id, page).await?;
    Ok(Some(TopicPageBody {
        forum: (&forum).into(),
        topic: (&topic).into(),
        posts: rows.into_iter().map(PostView::from).collect(),
    }))
}

/// 第 1 页走版本化缓存（plan 第 8 节「主题第 1 页」）。
async fn page1(state: &AppState, topic_id: i64) -> ApiResult<Option<TopicPageBody>> {
    let name = forum_cache::topic_page1(topic_id);
    let (version, cached) = state.cache.versioned_get(&name).await?;
    if let Some(body) = cached.and_then(|v| serde_json::from_str(&v).ok()) {
        return Ok(Some(body));
    }
    let body = build_body(state, topic_id, 1).await?;
    if let Some(b) = &body {
        let json = serde_json::to_string(b).map_err(ApiError::internal)?;
        state
            .cache
            .versioned_set(&name, &version, &json, PAGE1_TTL)
            .await?;
    }
    Ok(body)
}

/// 主题某一页。按楼层区间取（`floor BETWEEN`），第 1 页与第 5000 页一样快。
#[utoipa::path(
    get, path = "/api/v1/topics/{id}", tag = "forum",
    params(("id" = i64, Path), TopicQuery),
    responses((status = 200, body = TopicPage), (status = 400, body = ErrorBody), (status = 404, body = ErrorBody))
)]
pub async fn get(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
    Query(q): Query<TopicQuery>,
) -> ApiResult<Json<TopicPage>> {
    let page = q.page.unwrap_or(1);
    if page < 1 {
        return Err(ApiError(ErrorCode::InvalidParameter));
    }
    let viewer = Viewer::load(&state, &session).await?;
    let body = if page == 1 {
        page1(&state, id).await?
    } else {
        build_body(&state, id, page).await?
    }
    .ok_or(ApiError(ErrorCode::NotFound))?;

    let level = viewer.level();
    let staff = viewer.is_staff(&state, body.forum.id).await?;
    let flags = body.topic.flags;
    if (level < body.forum.view_min_level || flags.hidden) && !staff {
        return Err(ApiError(ErrorCode::NotFound));
    }
    let can_reply =
        viewer.user.is_some() && (staff || (!flags.locked && level >= body.forum.reply_min_level));
    let total_pages = (body.topic.last_floor + PAGE_SIZE - 1) / PAGE_SIZE;
    let user_ids: Vec<i64> = [body.topic.author_id]
        .into_iter()
        .chain(body.posts.iter().map(|p| p.author_id))
        .collect();
    let users = cards::load(&state, &user_ids).await?;
    after_commit("add_view", state.cache.add_view(id)).await;
    Ok(Json(TopicPage {
        body,
        users,
        page,
        total_pages,
        can_reply,
        can_moderate: staff,
    }))
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct LatestQuery {
    /// 上一页的 `next_cursor`。
    pub cursor: Option<String>,
}

/// 每次从有序集合多取一些：要按可见板块过滤掉一部分。
const LATEST_FETCH: usize = 2 * TOPICS_PER_PAGE;
/// 一次请求最多读几轮有序集合（看不到的板块主题很多时，返回不满一页也给出游标）。
const LATEST_ROUNDS: usize = 5;

/// 跨板块最新主题（按最后回复时间）。最新 1 万个取自 Valkey 有序集合（发帖、回帖时维护，与板块列表同一脚本），
/// 更早的按 `(last_post_at, id)` 游标走部分索引 `topics_latest`。
#[utoipa::path(
    get, path = "/api/v1/topics/latest", tag = "forum",
    params(LatestQuery),
    responses((status = 200, body = LatestResponse), (status = 400, body = ErrorBody))
)]
pub async fn latest_topics(
    State(state): State<AppState>,
    session: Session,
    Query(q): Query<LatestQuery>,
) -> ApiResult<Json<LatestResponse>> {
    let viewer = Viewer::load(&state, &session).await?;
    let forums: HashMap<i32, Forum> = visible_forums(&state, &viewer)
        .await?
        .into_iter()
        .map(|f| (f.id, f))
        .collect();
    // 扫描位置（不含）：上一页最后一个主题，之后是本次最后检查过的有序集合成员
    let mut pos = q
        .cursor
        .as_deref()
        .map(parse_cursor)
        .transpose()?
        .map(|(at, id)| (micros(at), id));
    let mut out: Vec<Topic> = Vec::new();
    // None：有序集合还没读完；Some(true)：读完了且数据库里还有更早的；Some(false)：全部读完
    let mut zset_end: Option<bool> = None;
    for _ in 0..LATEST_ROUNDS {
        let page = latest_zset(&state, pos.map(|p| p.0)).await?;
        // 分数相同的成员按字符串排序：取回后按 (分数, ID) 重新排序并按游标过滤
        let mut entries: Vec<(i64, i64)> = page
            .entries
            .iter()
            .map(|&(id, score)| (score, id))
            .filter(|&e| pos.is_none_or(|p| e < p))
            .collect();
        entries.sort_unstable_by(|a, b| b.cmp(a));
        let ids: Vec<i64> = entries.iter().map(|&(_, id)| id).collect();
        let mut rows: HashMap<i64, Topic> = topics::by_ids(&state.db, &ids)
            .await?
            .into_iter()
            .map(|t| (t.id, t))
            .collect();
        for e in entries {
            pos = Some(e);
            // 有序集合可能残留刚隐藏、置顶或移走的主题：按当前状态过滤
            if let Some(t) = rows
                .remove(&e.1)
                .filter(|t| t.listed() && forums.contains_key(&t.forum_id))
            {
                out.push(t);
                if out.len() == TOPICS_PER_PAGE {
                    break;
                }
            }
        }
        if page.entries.len() < LATEST_FETCH || ids.is_empty() {
            zset_end = Some(page.total >= BOARD_CAP as u64);
        }
        if out.len() == TOPICS_PER_PAGE || zset_end.is_some() {
            break;
        }
    }
    let mut more = out.len() == TOPICS_PER_PAGE || zset_end.is_none();
    if out.len() < TOPICS_PER_PAGE
        && zset_end == Some(true)
        && let Some((score, id)) = pos
    {
        let before = OffsetDateTime::from_unix_timestamp_nanos(i128::from(score) * 1000)
            .map_err(|_| ApiError(ErrorCode::InvalidParameter))?;
        let ids: Vec<i32> = forums.keys().copied().collect();
        let need = TOPICS_PER_PAGE - out.len();
        let rows = topics::latest_before(&state.db, &ids, (before, id), need as i64).await?;
        more = rows.len() == need;
        if let Some(t) = rows.last() {
            pos = Some((micros(t.last_post_at), t.id));
        }
        out.extend(rows);
    }
    let next_cursor = pos.filter(|_| more).map(|(s, id)| cursor_of(s, id));
    let mut forum_ids: Vec<i32> = out.iter().map(|t| t.forum_id).collect();
    forum_ids.sort_unstable();
    forum_ids.dedup();
    let user_ids = topic_user_ids(out.iter());
    Ok(Json(LatestResponse {
        topics: out.iter().map(TopicSummary::from).collect(),
        forums: forum_ids
            .iter()
            .filter_map(|id| forums.get(id))
            .map(ForumView::from)
            .collect(),
        users: cards::load(&state, &user_ids).await?,
        next_cursor,
    }))
}

/// 读一批跨板块最新有序集合；还没建好时从数据库重建（合并写入，与并发的发帖不会互相覆盖）。
async fn latest_zset(state: &AppState, max_score: Option<i64>) -> ApiResult<BoardPage> {
    if let Some(p) = state.cache.latest_page(max_score, LATEST_FETCH).await? {
        return Ok(p);
    }
    let rows = topics::latest_listed_all(&state.db, BOARD_CAP as i64).await?;
    let entries: Vec<(i64, i64)> = rows.iter().map(|&(id, at)| (id, micros(at))).collect();
    state.cache.latest_fill(&entries).await?;
    Ok(state
        .cache
        .latest_page(max_score, LATEST_FETCH)
        .await?
        .unwrap_or(BoardPage {
            entries: Vec::new(),
            total: 0,
        }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ReplyRequest {
    pub html: String,
    #[schema(value_type = Object)]
    pub source: Value,
}

/// 回帖。楼层在一条多 CTE 语句里分配，HTML 在事务外过滤（plan 第 8 节）。
#[utoipa::path(
    post, path = "/api/v1/topics/{id}/posts", tag = "forum",
    params(("id" = i64, Path)),
    request_body = ReplyRequest,
    responses(
        (status = 201, body = CreatedPost),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody), (status = 429, body = ErrorBody)
    )
)]
pub async fn reply(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
    Json(req): Json<ReplyRequest>,
) -> ApiResult<(StatusCode, Json<CreatedPost>)> {
    let viewer = Viewer::load(&state, &session).await?;
    let actor = viewer.require()?;
    let topic = topics::by_id(&state.db, id)
        .await?
        .ok_or(ApiError(ErrorCode::NotFound))?;
    let forum = forum_db::forums::by_id(&state.db, topic.forum_id)
        .await?
        .ok_or(ApiError(ErrorCode::NotFound))?;
    let hidden = topic.flags & FLAG_HIDDEN != 0;
    let locked = topic.flags & FLAG_LOCKED != 0;
    let needs_staff = hidden
        || locked
        || actor.level < forum.view_min_level
        || actor.level < forum.reply_min_level;
    let staff = needs_staff && viewer.is_staff(&state, forum.id).await?;
    if !staff {
        if hidden || actor.level < forum.view_min_level {
            return Err(ApiError(ErrorCode::NotFound));
        }
        if locked {
            return Err(ApiError(ErrorCode::TopicLocked));
        }
        if actor.level < forum.reply_min_level {
            return Err(ApiError(ErrorCode::Forbidden));
        }
    }
    posting::check_sanctions(&state, actor.id, Some(forum.id)).await?;
    let cleaned = posting::clean(&state, &actor, &req.html, &req.source)?;
    posting::limit_new_content(&state, &actor, &REPLY_RATE).await?;

    let replied = posts::reply(
        &state.db,
        posts::NewReply {
            topic_id: id,
            author_id: actor.id,
            html: &cleaned.html,
            plain_text: &cleaned.plain_text,
            source: &req.source,
            mentions: &cleaned.mentions,
            allow_locked: staff,
        },
    )
    .await?;
    // 检查之后到写入之间主题被锁定、隐藏或删除
    let Some(r) = replied else {
        let now = topics::by_id(&state.db, id).await?;
        return Err(ApiError(match now {
            Some(t) if t.flags & FLAG_HIDDEN == 0 => ErrorCode::TopicLocked,
            _ => ErrorCode::NotFound,
        }));
    };
    if r.flags & topics::FLAGS_NOT_LISTED == 0 {
        after_commit(
            "board_touch",
            state
                .cache
                .board_touch(r.forum_id, id, micros(r.last_post_at)),
        )
        .await;
    }
    after_commit(
        "bump page1",
        state.cache.bump(&forum_cache::topic_page1(id)),
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(CreatedPost {
            post_id: r.post_id,
            floor: r.floor,
            page: (r.floor - 1) / PAGE_SIZE + 1,
        }),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModerationAction {
    Pin,
    Unpin,
    Feature,
    Unfeature,
    Lock,
    Unlock,
    Hide,
    Unhide,
    Move,
}

impl ModerationAction {
    fn name(self) -> &'static str {
        match self {
            Self::Pin => "topic.pin",
            Self::Unpin => "topic.unpin",
            Self::Feature => "topic.feature",
            Self::Unfeature => "topic.unfeature",
            Self::Lock => "topic.lock",
            Self::Unlock => "topic.unlock",
            Self::Hide => "topic.hide",
            Self::Unhide => "topic.unhide",
            Self::Move => "topic.move",
        }
    }

    /// 需要的权限与限速类别：隐藏算删除类（计入异常检测），其余算内容类。
    fn permission(self) -> (Permission, Class) {
        match self {
            Self::Hide => (Permission::TopicHide, Class::Delete),
            Self::Unhide => (Permission::TopicHide, Class::Content),
            _ => (Permission::TopicModerate, Class::Content),
        }
    }

    /// `(要设置的位, 要清除的位)`。
    fn bits(self) -> (i16, i16) {
        match self {
            Self::Pin => (FLAG_PINNED, 0),
            Self::Unpin => (0, FLAG_PINNED),
            Self::Feature => (FLAG_FEATURED, 0),
            Self::Unfeature => (0, FLAG_FEATURED),
            Self::Lock => (FLAG_LOCKED, 0),
            Self::Unlock => (0, FLAG_LOCKED),
            Self::Hide => (FLAG_HIDDEN, 0),
            Self::Unhide => (0, FLAG_HIDDEN),
            Self::Move => (0, 0),
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ModerationRequest {
    pub action: ModerationAction,
    /// 预设理由或补充说明，写入审计日志（plan 第 9 节）。
    pub reason: String,
    /// `move` 的目标板块。
    pub forum_id: Option<i32>,
}

/// 主题管理。需要该板块的管理权限（版主只管自己的板块，移动时目标板块也要有权限）；
/// 限速、异常检测，每次操作写只增审计日志（ADR 0013）。
#[utoipa::path(
    post, path = "/api/v1/topics/{id}/moderation", tag = "forum",
    params(("id" = i64, Path)),
    request_body = ModerationRequest,
    responses(
        (status = 200, body = ModerationResult),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody), (status = 429, body = ErrorBody)
    )
)]
pub async fn moderate(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
    Json(req): Json<ModerationRequest>,
) -> ApiResult<Json<ModerationResult>> {
    let viewer = Viewer::load(&state, &session).await?;
    let actor = viewer.require()?;
    let reason = moderation::reason(&req.reason)?;
    let topic = topics::by_id(&state.db, id)
        .await?
        .ok_or(ApiError(ErrorCode::NotFound))?;
    let (permission, class) = req.action.permission();
    let m = Moderator::authorize(
        &state,
        actor.id,
        permission,
        Scope::Board(topic.forum_id),
        class,
    )
    .await?;
    let change = match (req.action, req.forum_id) {
        (ModerationAction::Move, Some(to)) if to != topic.forum_id => {
            forum_db::forums::by_id(&state.db, to)
                .await?
                .ok_or(ApiError(ErrorCode::NotFound))?;
            m.require(Permission::TopicModerate, Scope::Board(to))?;
            TopicChange::Move(to)
        }
        (ModerationAction::Move, _) => return Err(ApiError(ErrorCode::InvalidParameter)),
        (a, _) => {
            let (set, clear) = a.bits();
            TopicChange::Flags { set, clear }
        }
    };

    let mut tx = state.db.begin().await?;
    m.guard(&state, &mut tx).await?;
    let effect = moderation::change_topic(&mut tx, &topic, change)
        .await?
        .ok_or(ApiError(ErrorCode::NotFound))?;
    forum_db::audit::append(
        &mut *tx,
        actor.id,
        req.action.name(),
        Some(&format!("topic:{id}")),
        Some(reason),
        &effect.detail(),
    )
    .await?;
    tx.commit().await?;
    effect.apply(&state).await;

    Ok(Json(ModerationResult {
        forum_id: effect.new_forum,
        flags: effect.new_flags.into(),
    }))
}
