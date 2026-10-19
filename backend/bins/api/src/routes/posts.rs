//! 楼层：编辑（保留历史）、删除（保留楼层）与恢复、编辑历史、编辑器原稿。

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use forum_core::ErrorCode;
use forum_core::staff::{Permission, Scope};
use forum_db::posts::{self, PAGE_SIZE, Post, STATUS_DELETED_BY_AUTHOR, STATUS_DELETED_BY_STAFF};
use forum_db::topics::{self, FLAG_HIDDEN, FLAG_LOCKED, Topic};
use forum_db::users::Actor;
use forum_kv::forum_cache;
use serde::Deserialize;
use serde_json::{Value, json};
use tower_sessions::Session;
use utoipa::{IntoParams, ToSchema};

use crate::access::Viewer;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::moderation::{self, Class, Moderator, action};
use crate::posting::{self, EDIT_RATE, after_commit};
use crate::state::AppState;
use crate::views::{EditHistory, EditView, PostSource};

const HISTORY_PAGE: i64 = 20;

/// 楼层所在的主题，以及访问者能否以管理人员身份进入该板块。
struct Ctx {
    post: Post,
    topic: Topic,
    staff: bool,
}

/// 取楼层并检查可见性（隐藏主题、等级不够的板块对非管理人员表现为不存在）。
async fn load(state: &AppState, viewer: &Viewer, post_id: i64) -> ApiResult<Ctx> {
    let post = posts::by_id(&state.db, post_id)
        .await?
        .ok_or(ApiError(ErrorCode::NotFound))?;
    let topic = topics::by_id(&state.db, post.topic_id)
        .await?
        .ok_or(ApiError(ErrorCode::NotFound))?;
    let forum = forum_db::forums::by_id(&state.db, topic.forum_id)
        .await?
        .ok_or(ApiError(ErrorCode::NotFound))?;
    let staff = viewer.is_staff(state, forum.id).await?;
    if !staff && (topic.flags & FLAG_HIDDEN != 0 || viewer.level() < forum.view_min_level) {
        return Err(ApiError(ErrorCode::NotFound));
    }
    Ok(Ctx { post, topic, staff })
}

/// 改动未删除的楼层：作者本人（锁定的主题只有管理人员能改，被禁言时不能改）直接放行，返回 `None`；
/// 他人的楼层需要板块内的 `permission`，返回已通过权限与限速的操作者。
async fn writer(
    state: &AppState,
    ctx: &Ctx,
    actor: &Actor,
    permission: Permission,
    class: Class,
) -> ApiResult<Option<Moderator>> {
    if ctx.post.status != posts::STATUS_NORMAL {
        return Err(ApiError(ErrorCode::NotFound));
    }
    if ctx.post.author_id == actor.id {
        if !ctx.staff && ctx.topic.flags & FLAG_LOCKED != 0 {
            return Err(ApiError(ErrorCode::TopicLocked));
        }
        posting::check_sanctions(state, actor.id, Some(ctx.topic.forum_id)).await?;
        return Ok(None);
    }
    Moderator::authorize(
        state,
        actor.id,
        permission,
        Scope::Board(ctx.topic.forum_id),
        class,
    )
    .await
    .map(Some)
}

async fn invalidate(state: &AppState, ctx: &Ctx, title_changed: bool) {
    if ctx.post.floor <= PAGE_SIZE || title_changed {
        after_commit(
            "bump page1",
            state.cache.bump(&forum_cache::topic_page1(ctx.topic.id)),
        )
        .await;
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct EditRequest {
    pub html: String,
    #[schema(value_type = Object)]
    pub source: Value,
    /// 只有 1 楼可以同时改标题。
    pub title: Option<String>,
}

/// 编辑楼层。编辑前的版本存入编辑历史；编辑他人楼层需要管理员权限（版主不能改他人内容），并写审计日志。
#[utoipa::path(
    patch, path = "/api/v1/posts/{id}", tag = "forum",
    params(("id" = i64, Path)),
    request_body = EditRequest,
    responses(
        (status = 204),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody), (status = 429, body = ErrorBody)
    )
)]
pub async fn edit(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
    Json(req): Json<EditRequest>,
) -> ApiResult<StatusCode> {
    let viewer = Viewer::load(&state, &session).await?;
    let actor = viewer.require()?;
    let ctx = load(&state, &viewer, id).await?;
    let m = writer(&state, &ctx, &actor, Permission::PostEdit, Class::Content).await?;
    let title = match req.title.as_deref() {
        Some(_) if ctx.post.floor != 1 => return Err(ApiError(ErrorCode::InvalidParameter)),
        Some(t) => Some(forum_content::clean_title(t)?).filter(|t| *t != ctx.topic.title),
        None => None,
    };
    let cleaned = posting::clean(&state, &actor, &req.html, &req.source)?;
    if m.is_none() {
        posting::limit(&state, &EDIT_RATE, actor.id).await?;
    }

    let mut tx = state.db.begin().await?;
    if let Some(m) = &m {
        m.guard(&state, &mut tx).await?;
    }
    let edit_id = posts::edit(
        &mut tx,
        posts::Edit {
            post_id: id,
            editor_id: actor.id,
            html: &cleaned.html,
            plain_text: &cleaned.plain_text,
            source: &req.source,
            old_title: title.as_ref().map(|_| ctx.topic.title.as_str()),
        },
    )
    .await?
    .ok_or(ApiError(ErrorCode::NotFound))?;
    if let Some(t) = &title {
        topics::set_title(&mut *tx, ctx.topic.id, t).await?;
    }
    if m.is_some() {
        forum_db::audit::append(
            &mut *tx,
            actor.id,
            action::POST_EDIT,
            Some(&format!("post:{id}")),
            None,
            &json!({ "topic_id": ctx.topic.id, "floor": ctx.post.floor, "edit_id": edit_id }),
        )
        .await?;
    }
    tx.commit().await?;
    invalidate(&state, &ctx, title.is_some()).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct DeleteQuery {
    /// 管理人员删除他人楼层时必填，写入审计日志。
    pub reason: Option<String>,
}

/// 删除楼层：只改状态，楼层号保留（plan 第 4 节）。删除他人楼层需要板块管理权限，计入删帖限速与异常检测。
#[utoipa::path(
    delete, path = "/api/v1/posts/{id}", tag = "forum",
    params(("id" = i64, Path), DeleteQuery),
    responses(
        (status = 204),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody), (status = 429, body = ErrorBody)
    )
)]
pub async fn delete(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
    Query(q): Query<DeleteQuery>,
) -> ApiResult<StatusCode> {
    let viewer = Viewer::load(&state, &session).await?;
    let actor = viewer.require()?;
    let ctx = load(&state, &viewer, id).await?;
    let m = writer(&state, &ctx, &actor, Permission::PostDelete, Class::Delete).await?;
    let reason = match &m {
        Some(_) => Some(moderation::reason(q.reason.as_deref().unwrap_or_default())?),
        None => None,
    };

    let mut tx = state.db.begin().await?;
    if let Some(m) = &m {
        m.guard(&state, &mut tx).await?;
    }
    let status = if m.is_none() {
        STATUS_DELETED_BY_AUTHOR
    } else {
        STATUS_DELETED_BY_STAFF
    };
    if !posts::soft_delete(&mut *tx, id, status).await? {
        return Err(ApiError(ErrorCode::NotFound));
    }
    if m.is_some() {
        forum_db::audit::append(
            &mut *tx,
            actor.id,
            action::POST_DELETE,
            Some(&format!("post:{id}")),
            reason,
            &json!({ "topic_id": ctx.topic.id, "floor": ctx.post.floor, "author_id": ctx.post.author_id }),
        )
        .await?;
    }
    tx.commit().await?;
    invalidate(&state, &ctx, false).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RestoreRequest {
    /// 写入审计日志。
    pub reason: String,
}

/// 恢复被管理人员删除的楼层（删帖的反向操作）。需要板块管理权限。
#[utoipa::path(
    post, path = "/api/v1/posts/{id}/restore", tag = "forum",
    params(("id" = i64, Path)),
    request_body = RestoreRequest,
    responses(
        (status = 204),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody), (status = 429, body = ErrorBody)
    )
)]
pub async fn restore(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
    Json(req): Json<RestoreRequest>,
) -> ApiResult<StatusCode> {
    let viewer = Viewer::load(&state, &session).await?;
    let actor = viewer.require()?;
    let reason = moderation::reason(&req.reason)?;
    let ctx = load(&state, &viewer, id).await?;
    let m = Moderator::authorize(
        &state,
        actor.id,
        Permission::PostDelete,
        Scope::Board(ctx.topic.forum_id),
        Class::Content,
    )
    .await?;
    let mut tx = state.db.begin().await?;
    m.guard(&state, &mut tx).await?;
    if !posts::restore(&mut *tx, id).await? {
        return Err(ApiError(ErrorCode::NotFound));
    }
    forum_db::audit::append(
        &mut *tx,
        actor.id,
        action::POST_RESTORE,
        Some(&format!("post:{id}")),
        Some(reason),
        &json!({ "topic_id": ctx.topic.id, "floor": ctx.post.floor }),
    )
    .await?;
    tx.commit().await?;
    invalidate(&state, &ctx, false).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct HistoryQuery {
    /// 游标：上一页的 `next_cursor`。
    pub cursor: Option<i64>,
}

/// 编辑历史（编辑前的各个版本），新的在前。
#[utoipa::path(
    get, path = "/api/v1/posts/{id}/history", tag = "forum",
    params(("id" = i64, Path), HistoryQuery),
    responses((status = 200, body = EditHistory), (status = 404, body = ErrorBody))
)]
pub async fn history(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
    Query(q): Query<HistoryQuery>,
) -> ApiResult<Json<EditHistory>> {
    let viewer = Viewer::load(&state, &session).await?;
    let ctx = load(&state, &viewer, id).await?;
    // 已删除楼层的历史只有管理人员能看
    if ctx.post.status != posts::STATUS_NORMAL && !ctx.staff {
        return Err(ApiError(ErrorCode::NotFound));
    }
    let rows = posts::history(&state.db, id, q.cursor, HISTORY_PAGE).await?;
    let next_cursor = (rows.len() as i64 == HISTORY_PAGE)
        .then(|| rows.last().map(|r| r.id.to_string()))
        .flatten();
    Ok(Json(EditHistory {
        edits: rows
            .into_iter()
            .map(|r| EditView {
                id: r.id,
                editor_id: r.editor_id,
                title: r.title,
                html: r.html,
                edited_at: r.edited_at,
            })
            .collect(),
        next_cursor,
    }))
}

/// 编辑器原稿：作者本人或能编辑他人楼层的管理人员重新编辑时加载。
#[utoipa::path(
    get, path = "/api/v1/posts/{id}/source", tag = "forum",
    params(("id" = i64, Path)),
    responses(
        (status = 200, body = PostSource),
        (status = 401, body = ErrorBody), (status = 403, body = ErrorBody), (status = 404, body = ErrorBody)
    )
)]
pub async fn source(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
) -> ApiResult<Json<PostSource>> {
    let viewer = Viewer::load(&state, &session).await?;
    let actor = viewer.require()?;
    let ctx = load(&state, &viewer, id).await?;
    if ctx.post.status != posts::STATUS_NORMAL {
        return Err(ApiError(ErrorCode::NotFound));
    }
    if ctx.post.author_id != actor.id {
        let roles = viewer.staff(&state).await?;
        if !roles.allows(Permission::PostEdit, Scope::Board(ctx.topic.forum_id)) {
            return Err(moderation::denied(&roles));
        }
    }
    let source = posts::source(&state.db, id)
        .await?
        .ok_or(ApiError(ErrorCode::NotFound))?;
    Ok(Json(PostSource { source }))
}
