//! 主题页、回帖、主题管理（置顶 / 精华 / 锁定 / 隐藏 / 移动）。

use std::collections::HashSet;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use forum_core::ErrorCode;
use forum_db::posts::{self, PAGE_SIZE};
use forum_db::topics::{self, FLAG_FEATURED, FLAG_HIDDEN, FLAG_LOCKED, FLAG_PINNED};
use forum_kv::forum_cache;
use serde::Deserialize;
use serde_json::{Value, json};
use tower_sessions::Session;
use utoipa::{IntoParams, ToSchema};

use crate::access::Viewer;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::posting::{self, REPLY_RATE, after_commit, micros};
use crate::state::AppState;
use crate::views::{
    CreatedPost, ModerationResult, PostView, TopicPage, TopicPageBody, UserCardView,
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
    let user_ids: Vec<i64> = rows
        .iter()
        .map(|p| p.author_id)
        .chain([topic.author_id])
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let users = forum_db::users::cards(&state.db, &user_ids).await?;
    Ok(Some(TopicPageBody {
        forum: (&forum).into(),
        topic: (&topic).into(),
        posts: rows.into_iter().map(PostView::from).collect(),
        users: users.into_iter().map(UserCardView::from).collect(),
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
    after_commit("add_view", state.cache.add_view(id)).await;
    Ok(Json(TopicPage {
        body,
        page,
        total_pages,
        can_reply,
        can_moderate: staff,
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

/// 主题管理。只有该板块的版主和全站管理人员可操作；每次操作写只增审计日志。
#[utoipa::path(
    post, path = "/api/v1/topics/{id}/moderation", tag = "forum",
    params(("id" = i64, Path)),
    request_body = ModerationRequest,
    responses(
        (status = 200, body = ModerationResult),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody)
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
    let reason = req.reason.trim();
    if reason.is_empty() || reason.chars().count() > 500 {
        return Err(ApiError(ErrorCode::InvalidParameter));
    }
    let topic = topics::by_id(&state.db, id)
        .await?
        .ok_or(ApiError(ErrorCode::NotFound))?;
    if !viewer.is_staff(&state, topic.forum_id).await? {
        return Err(ApiError(ErrorCode::Forbidden));
    }
    let target_forum = match (req.action, req.forum_id) {
        (ModerationAction::Move, Some(to)) if to != topic.forum_id => {
            forum_db::forums::by_id(&state.db, to)
                .await?
                .ok_or(ApiError(ErrorCode::NotFound))?;
            if !viewer.is_staff(&state, to).await? {
                return Err(ApiError(ErrorCode::Forbidden));
            }
            Some(to)
        }
        (ModerationAction::Move, _) => return Err(ApiError(ErrorCode::InvalidParameter)),
        _ => None,
    };

    let mut tx = state.db.begin().await?;
    let (old_forum, new_forum, old_flags, new_flags, detail) = if let Some(to) = target_forum {
        let from = topics::move_to(&mut *tx, id, to)
            .await?
            .ok_or(ApiError(ErrorCode::NotFound))?;
        (
            from,
            to,
            topic.flags,
            topic.flags,
            json!({ "from": from, "to": to }),
        )
    } else {
        let (set, clear) = req.action.bits();
        let (old, new, forum) = topics::update_flags(&mut *tx, id, set, clear)
            .await?
            .ok_or(ApiError(ErrorCode::NotFound))?;
        (
            forum,
            forum,
            old,
            new,
            json!({ "old_flags": old, "new_flags": new }),
        )
    };
    forum_db::audit::append(
        &mut *tx,
        actor.id,
        req.action.name(),
        Some(&format!("topic:{id}")),
        Some(reason),
        &detail,
    )
    .await?;
    tx.commit().await?;

    // 缓存：有序集合成员关系、置顶列表、第 1 页
    let was_listed = old_flags & topics::FLAGS_NOT_LISTED == 0;
    let is_listed = new_flags & topics::FLAGS_NOT_LISTED == 0;
    if was_listed && (!is_listed || old_forum != new_forum) {
        after_commit("board_remove", state.cache.board_remove(old_forum, id)).await;
    }
    if is_listed && (!was_listed || old_forum != new_forum) {
        after_commit(
            "board_touch",
            state
                .cache
                .board_touch(new_forum, id, micros(topic.last_post_at)),
        )
        .await;
    }
    after_commit(
        "bump pinned",
        state.cache.bump(&forum_cache::pinned(old_forum)),
    )
    .await;
    if new_forum != old_forum {
        after_commit(
            "bump pinned",
            state.cache.bump(&forum_cache::pinned(new_forum)),
        )
        .await;
    }
    after_commit(
        "bump page1",
        state.cache.bump(&forum_cache::topic_page1(id)),
    )
    .await;

    Ok(Json(ModerationResult {
        forum_id: new_forum,
        flags: new_flags.into(),
    }))
}
