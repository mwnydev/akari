//! 楼层下方的朋友圈式社交区（设计稿 v2）：点赞、两层评论（楼层 → 平铺评论，回复评论只记下被回复的人）、
//! 举报楼层与评论；主题页每楼的点赞与评论概况也在这里组装。
//!
//! - 点赞：登录即可（被封号的不能），幂等；楼主收到合并的点赞通知（窗口内的赞合成一条，见 [`LIKE_DIGEST_WINDOW`]）。
//! - 评论：与回帖相同的门槛（回复等级、锁帖、禁言 / 封号、新用户限频），纯文本 1–500 字，`@用户名` 走提及通知。
//!   作者本人可删除；他人的评论需要板块的删帖权限（版主限本板块），要填理由、写审计日志，可恢复、可一键撤销。
//! - 打开主题页的连接（WebSocket 订阅 `topic_id`）实时收到新评论、删除评论与点赞数变化；隐藏的主题不推送。

use std::collections::HashMap;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use forum_core::ErrorCode;
use forum_core::chat::Event;
use forum_core::floor::{PREVIEW_COMMENTS, PREVIEW_LIKERS};
use forum_core::staff::{Permission, Scope};
use forum_db::floor::{self, CommentRow, FloorComment};
use forum_db::posts::STATUS_NORMAL;
use forum_db::reports::{self, TARGET_COMMENT, TARGET_POST, Target};
use forum_db::topics::{FLAG_HIDDEN, FLAG_LOCKED, Topic};
use forum_db::{Postgres, Transaction};
use forum_kv::realtime::{Route, Topic as RealtimeTopic};
use serde::Deserialize;
use serde_json::json;
use tower_sessions::Session;
use utoipa::{IntoParams, ToSchema};

use crate::access::Viewer;
use crate::cards;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::guest;
use crate::middleware::ClientIp;
use crate::moderation::{self, Class, Moderator, action};
use crate::posting::{self, COMMENT_RATE, LIKE_RATE, REPORT_RATE};
use crate::routes::posts::{Ctx, load};
use crate::state::AppState;
use crate::views::{CommentPage, CreatedComment, FloorSocialView, LikeState, PostView};

/// 合并点赞通知的窗口：第一个赞之后这么久投递，期间的赞合成一条。
pub const LIKE_DIGEST_WINDOW: Duration = Duration::from_secs(300);
/// 评论翻页每页条数。
const COMMENT_PAGE: i64 = 50;
/// 举报理由最多字数（可以不填）。
const MAX_REPORT_REASON_CHARS: usize = 500;

/// 本页各楼的点赞与评论概况（顺序同 `posts`），以及其中出现的用户（点赞名单、评论作者与被回复的人）。
/// 已删除的楼层、访问者看不到的隐藏楼层给空概况，不查也不返回它的评论。两次批量查询，与楼层数无关。
pub async fn page_social(
    state: &AppState,
    posts: &[PostView],
    staff: bool,
    viewer: Option<i64>,
) -> ApiResult<(Vec<FloorSocialView>, Vec<i64>)> {
    let live: Vec<i64> = posts
        .iter()
        .filter(|p| p.readable(staff, viewer))
        .map(|p| p.id)
        .collect();
    let mut social: HashMap<i64, floor::Social> =
        floor::social(&state.db, &live, viewer, PREVIEW_LIKERS)
            .await?
            .into_iter()
            .map(|s| (s.post_id, s))
            .collect();
    let mut comments: HashMap<i64, Vec<FloorComment>> = HashMap::new();
    for c in floor::preview(&state.db, &live, PREVIEW_COMMENTS).await? {
        comments.entry(c.post_id).or_default().push(c);
    }
    let mut users = Vec::new();
    let views = posts
        .iter()
        .map(|p| {
            let s = social.remove(&p.id).unwrap_or(floor::Social {
                post_id: p.id,
                likes: 0,
                comments: 0,
                likers: Vec::new(),
                liked: false,
            });
            let comments = comments.remove(&p.id).unwrap_or_default();
            users.extend(&s.likers);
            users.extend(
                comments
                    .iter()
                    .flat_map(|c| std::iter::once(c.author_id).chain(c.reply_to_id)),
            );
            FloorSocialView {
                post_id: p.id,
                likes: s.likes,
                liked: s.liked,
                likers: s.likers,
                comment_count: s.comments,
                comments,
            }
        })
        .collect();
    Ok((views, users))
}

/// 推给打开着这个主题页的连接。隐藏的主题不推（订阅时还看得到的人此后不应再收到任何变化）。
/// 点赞与评论的每次变化都经过这里，游客缓存里的主题页也在这里失效。
async fn push(state: &AppState, topic: &Topic, event: &Event) {
    guest::topic_changed(state, topic.id).await;
    if topic.flags & FLAG_HIDDEN != 0 {
        return;
    }
    state
        .hub
        .publish(
            Route::Watchers {
                topic: RealtimeTopic::Thread(topic.id),
            },
            event,
        )
        .await;
}

/// 取楼层并要求未删除、主题未隐藏：已删除的楼层与隐藏的主题对所有人（含管理人员）都不能点赞、评论、举报，
/// 与隐藏主题不能回帖一致。
async fn live_post(state: &AppState, viewer: &Viewer, post_id: i64) -> ApiResult<Ctx> {
    let ctx = load(state, viewer, post_id).await?;
    if ctx.post.status != STATUS_NORMAL || ctx.topic.flags & FLAG_HIDDEN != 0 {
        return Err(ApiError(ErrorCode::NotFound));
    }
    Ok(ctx)
}

/// 点赞（重复点赞不报错）。被封号的不能点赞。
#[utoipa::path(
    put, path = "/api/v1/posts/{id}/like", tag = "forum", operation_id = "like_post",
    params(("id" = i64, Path)),
    responses(
        (status = 200, body = LikeState),
        (status = 401, body = ErrorBody), (status = 403, body = ErrorBody), (status = 404, body = ErrorBody),
        (status = 429, body = ErrorBody)
    )
)]
pub async fn like(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
) -> ApiResult<Json<LikeState>> {
    let viewer = Viewer::load(&state, &session).await?;
    let actor = viewer.require()?;
    let ctx = live_post(&state, &viewer, id).await?;
    posting::check_not_banned(&state, actor.id).await?;
    posting::limit(&state, &LIKE_RATE, actor.id).await?;
    let r = floor::like(
        &state.db,
        id,
        actor.id,
        ctx.post.author_id != actor.id,
        LIKE_DIGEST_WINDOW.as_secs_f64(),
    )
    .await?;
    if !r.found {
        return Err(ApiError(ErrorCode::NotFound));
    }
    if r.changed {
        push_likes(&state, &ctx.topic, id, actor.id, true, r.likes).await;
    }
    Ok(Json(LikeState {
        liked: true,
        likes: r.likes,
    }))
}

/// 取消赞（没赞过也不报错）。楼层已删除时也可以取消。
#[utoipa::path(
    delete, path = "/api/v1/posts/{id}/like", tag = "forum", operation_id = "unlike_post",
    params(("id" = i64, Path)),
    responses(
        (status = 200, body = LikeState),
        (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 429, body = ErrorBody)
    )
)]
pub async fn unlike(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
) -> ApiResult<Json<LikeState>> {
    let viewer = Viewer::load(&state, &session).await?;
    let actor = viewer.require()?;
    let ctx = load(&state, &viewer, id).await?;
    posting::limit(&state, &LIKE_RATE, actor.id).await?;
    let r = floor::unlike(&state.db, id, actor.id).await?;
    if r.changed {
        push_likes(&state, &ctx.topic, id, actor.id, false, r.likes).await;
    }
    Ok(Json(LikeState {
        liked: false,
        likes: r.likes,
    }))
}

async fn push_likes(
    state: &AppState,
    topic: &Topic,
    post_id: i64,
    user_id: i64,
    liked: bool,
    likes: i32,
) {
    let event = Event::Likes {
        topic_id: topic.id,
        post_id,
        user_id,
        liked,
        likes,
    };
    push(state, topic, &event).await;
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct CommentsQuery {
    /// 游标：已经显示的最后一条评论 ID（主题页给出的前 5 条之后从第 5 条的 ID 开始）。
    pub after: Option<i64>,
}

/// 一楼的评论（「展开其余」），按时间正序，每页 50 条。已删除楼层的评论只有管理人员能看，隐藏楼层的另有作者本人。
#[utoipa::path(
    get, path = "/api/v1/posts/{id}/comments", tag = "forum", operation_id = "post_comments",
    params(("id" = i64, Path), CommentsQuery),
    responses((status = 200, body = CommentPage), (status = 404, body = ErrorBody))
)]
pub async fn comments(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
    Query(q): Query<CommentsQuery>,
) -> ApiResult<Json<CommentPage>> {
    let viewer = Viewer::load(&state, &session).await?;
    let ctx = load(&state, &viewer, id).await?;
    if !ctx.readable(viewer.user.map(|u| u.id)) {
        return Err(ApiError(ErrorCode::NotFound));
    }
    let mut rows =
        floor::comments_after(&state.db, id, q.after.unwrap_or(0), COMMENT_PAGE + 1).await?;
    let more = rows.len() as i64 > COMMENT_PAGE;
    rows.truncate(COMMENT_PAGE as usize);
    let next_cursor = rows.last().map(|c| c.id).filter(|_| more);
    let user_ids: Vec<i64> = rows
        .iter()
        .flat_map(|c| std::iter::once(c.author_id).chain(c.reply_to_id))
        .collect();
    Ok(Json(CommentPage {
        users: cards::load(&state, &user_ids).await?,
        comments: rows,
        next_cursor,
    }))
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NewCommentRequest {
    /// 纯文本，1–500 字（换行与控制字符换成空格）；`@用户名` 会通知对方。
    pub body: String,
    /// 回复这一楼里的某条评论（「回复 某人」）；为空表示直接评论楼层。
    pub reply_to: Option<i64>,
    /// 注册 24 小时内必填：Turnstile 令牌（组件的 `action` 为 `post`）。
    pub captcha: Option<String>,
}

/// 评论楼层。门槛与回帖相同：回复等级、锁帖（管理人员除外）、禁言 / 封号、新用户人机验证与限频、评论限频。
/// 通知楼主、被回复的人与被 @ 的人，并推给打开着这个主题页的连接。
#[utoipa::path(
    post, path = "/api/v1/posts/{id}/comments", tag = "forum", operation_id = "create_comment",
    params(("id" = i64, Path)),
    request_body = NewCommentRequest,
    responses(
        (status = 201, body = CreatedComment),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody), (status = 429, body = ErrorBody)
    )
)]
pub async fn create_comment(
    State(state): State<AppState>,
    session: Session,
    ClientIp(ip): ClientIp,
    Path(id): Path<i64>,
    Json(req): Json<NewCommentRequest>,
) -> ApiResult<(StatusCode, Json<CreatedComment>)> {
    let viewer = Viewer::load(&state, &session).await?;
    let actor = viewer.require()?;
    let ctx = live_post(&state, &viewer, id).await?;
    if !ctx.staff {
        if ctx.topic.flags & FLAG_LOCKED != 0 {
            return Err(ApiError(ErrorCode::TopicLocked));
        }
        if actor.level < ctx.forum.reply_min_level {
            return Err(ApiError(ErrorCode::Forbidden));
        }
    }
    posting::check_sanctions(&state, actor.id, Some(ctx.forum.id)).await?;
    let body = forum_content::clean_comment(&req.body)?;
    if let Some(r) = req.reply_to {
        let target = floor::comment(&state.db, r).await?;
        if !target.is_some_and(|c| c.post_id == id && c.status == floor::STATUS_NORMAL) {
            return Err(ApiError(ErrorCode::InvalidParameter));
        }
    }
    posting::check_new_user_captcha(&state, &actor, req.captcha.as_deref(), ip).await?;
    posting::limit_new_content(&state, &actor, &COMMENT_RATE).await?;
    let mentions = posting::comment_mentions(&state, &body).await?;

    let (comment, comment_count) = floor::create(
        &state.db,
        floor::NewComment {
            post_id: id,
            author_id: actor.id,
            reply_to: req.reply_to,
            body: &body,
            mentions: &mentions,
        },
    )
    .await?
    // 检查之后楼层或被回复的评论被删除
    .ok_or(ApiError(ErrorCode::NotFound))?;
    let event = Event::Comment {
        topic_id: ctx.topic.id,
        comment: comment.clone(),
        comments: comment_count,
    };
    push(&state, &ctx.topic, &event).await;
    Ok((
        StatusCode::CREATED,
        Json(CreatedComment {
            comment,
            comment_count,
        }),
    ))
}

/// 评论与它所在的楼层；楼层与主题的可见性同楼层接口。
pub(crate) async fn load_comment(
    state: &AppState,
    viewer: &Viewer,
    id: i64,
) -> ApiResult<(CommentRow, Ctx)> {
    let c = floor::comment(&state.db, id)
        .await?
        .ok_or(ApiError(ErrorCode::NotFound))?;
    let ctx = load(state, viewer, c.post_id).await?;
    Ok((c, ctx))
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct DeleteCommentQuery {
    /// 管理人员删除他人评论时必填，写入审计日志。
    pub reason: Option<String>,
}

/// 删除评论（只改状态）。作者本人可删（锁定的主题只有管理人员能删，被禁言时不能删，同楼层）；
/// 他人的评论需要板块的删帖权限，计入删帖限速与异常检测，写审计日志（可恢复、可撤销）。
#[utoipa::path(
    delete, path = "/api/v1/comments/{id}", tag = "forum", operation_id = "delete_comment",
    params(("id" = i64, Path), DeleteCommentQuery),
    responses(
        (status = 204),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody), (status = 429, body = ErrorBody)
    )
)]
pub async fn delete_comment(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
    Query(q): Query<DeleteCommentQuery>,
) -> ApiResult<StatusCode> {
    let viewer = Viewer::load(&state, &session).await?;
    let actor = viewer.require()?;
    let (c, ctx) = load_comment(&state, &viewer, id).await?;
    if c.status != floor::STATUS_NORMAL {
        return Err(ApiError(ErrorCode::NotFound));
    }
    let m = if c.author_id == actor.id {
        if !ctx.staff && ctx.topic.flags & FLAG_LOCKED != 0 {
            return Err(ApiError(ErrorCode::TopicLocked));
        }
        posting::check_sanctions(&state, actor.id, Some(ctx.forum.id)).await?;
        None
    } else {
        let m = Moderator::authorize(
            &state,
            actor.id,
            Permission::PostDelete,
            Scope::Board(ctx.forum.id),
            Class::Delete,
        )
        .await?;
        let reason = moderation::reason(q.reason.as_deref().unwrap_or_default())?;
        Some((m, reason))
    };

    let mut tx = state.db.begin().await?;
    let comment_count = match &m {
        Some((m, reason)) => {
            m.guard(&state, &mut tx).await?;
            staff_delete_comment(&mut tx, actor.id, &c, ctx.topic.id, reason).await?
        }
        None => floor::soft_delete(&mut *tx, id, floor::STATUS_DELETED_BY_AUTHOR)
            .await?
            .ok_or(ApiError(ErrorCode::NotFound))?,
    };
    tx.commit().await?;
    push_comment_removed(&state, &ctx.topic, &c, comment_count).await;
    Ok(StatusCode::NO_CONTENT)
}

/// 管理人员删除他人评论（调用方已核对权限、限速，并在管理事务里做过异常检测）：软删除并写审计日志，
/// 返回该楼现在的评论数；评论已删除时返回 `NOT_FOUND`。删除接口与举报处理共用。
pub(crate) async fn staff_delete_comment(
    tx: &mut Transaction<'_, Postgres>,
    actor_id: i64,
    c: &CommentRow,
    topic_id: i64,
    reason: &str,
) -> ApiResult<i32> {
    let comment_count = floor::soft_delete(&mut **tx, c.id, floor::STATUS_DELETED_BY_STAFF)
        .await?
        .ok_or(ApiError(ErrorCode::NotFound))?;
    forum_db::audit::append(
        &mut **tx,
        actor_id,
        action::COMMENT_DELETE,
        Some(&format!("comment:{}", c.id)),
        Some(reason),
        &json!({ "post_id": c.post_id, "topic_id": topic_id, "author_id": c.author_id }),
    )
    .await?;
    Ok(comment_count)
}

/// 评论删除后推给打开着这个主题页的连接。
pub(crate) async fn push_comment_removed(
    state: &AppState,
    topic: &Topic,
    c: &CommentRow,
    comments: i32,
) {
    let event = Event::CommentRemoved {
        topic_id: topic.id,
        post_id: c.post_id,
        comment_id: c.id,
        comments,
    };
    push(state, topic, &event).await;
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RestoreCommentRequest {
    /// 写入审计日志。
    pub reason: String,
}

/// 恢复被管理人员删除的评论（删除的反向操作）。需要板块的删帖权限，写审计日志。
#[utoipa::path(
    post, path = "/api/v1/comments/{id}/restore", tag = "forum", operation_id = "restore_comment",
    params(("id" = i64, Path)),
    request_body = RestoreCommentRequest,
    responses(
        (status = 204),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody), (status = 429, body = ErrorBody)
    )
)]
pub async fn restore_comment(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
    Json(req): Json<RestoreCommentRequest>,
) -> ApiResult<StatusCode> {
    let viewer = Viewer::load(&state, &session).await?;
    let actor = viewer.require()?;
    let reason = moderation::reason(&req.reason)?;
    let (c, ctx) = load_comment(&state, &viewer, id).await?;
    let m = Moderator::authorize(
        &state,
        actor.id,
        Permission::PostDelete,
        Scope::Board(ctx.forum.id),
        Class::Content,
    )
    .await?;
    let mut tx = state.db.begin().await?;
    m.guard(&state, &mut tx).await?;
    let comment_count = floor::restore(&mut *tx, id)
        .await?
        .ok_or(ApiError(ErrorCode::NotFound))?;
    forum_db::audit::append(
        &mut *tx,
        actor.id,
        action::COMMENT_RESTORE,
        Some(&format!("comment:{id}")),
        Some(reason),
        &json!({ "post_id": c.post_id, "topic_id": ctx.topic.id }),
    )
    .await?;
    tx.commit().await?;
    if ctx.post.status == STATUS_NORMAL {
        let event = Event::Comment {
            topic_id: ctx.topic.id,
            comment: c.view(),
            comments: comment_count,
        };
        push(&state, &ctx.topic, &event).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReportRequest {
    /// 可以不填，最多 500 字。
    #[serde(default)]
    pub reason: String,
}

/// 记一条举报（进处理队列，同一内容待处理的举报归进同一件案件）：登录、未被封号、限频；
/// 重复举报同一内容不报错也不重复记。举报人的身份不会出现在被举报人能看到的任何地方。
async fn report(
    state: &AppState,
    reporter: i64,
    target: Target,
    reason: &str,
) -> ApiResult<StatusCode> {
    let reason = reason.trim();
    if reason.chars().count() > MAX_REPORT_REASON_CHARS {
        return Err(ApiError(ErrorCode::ContentTooLong));
    }
    posting::check_not_banned(state, reporter).await?;
    posting::limit(state, &REPORT_RATE, reporter).await?;
    let mut tx = state.db.begin().await?;
    if reports::create(&mut tx, target, reporter, reason).await? {
        tx.commit().await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// 举报楼层（进管理人员的处理队列，按楼层所在板块）。
#[utoipa::path(
    post, path = "/api/v1/posts/{id}/report", tag = "forum", operation_id = "report_post",
    params(("id" = i64, Path)),
    request_body = ReportRequest,
    responses(
        (status = 204),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody), (status = 429, body = ErrorBody)
    )
)]
pub async fn report_post(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
    Json(req): Json<ReportRequest>,
) -> ApiResult<StatusCode> {
    let viewer = Viewer::load(&state, &session).await?;
    let actor = viewer.require()?;
    let ctx = live_post(&state, &viewer, id).await?;
    let target = Target {
        kind: TARGET_POST,
        id,
        forum_id: ctx.forum.id,
        author_id: ctx.post.author_id,
    };
    report(&state, actor.id, target, &req.reason).await
}

/// 举报评论。
#[utoipa::path(
    post, path = "/api/v1/comments/{id}/report", tag = "forum", operation_id = "report_comment",
    params(("id" = i64, Path)),
    request_body = ReportRequest,
    responses(
        (status = 204),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody), (status = 429, body = ErrorBody)
    )
)]
pub async fn report_comment(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
    Json(req): Json<ReportRequest>,
) -> ApiResult<StatusCode> {
    let viewer = Viewer::load(&state, &session).await?;
    let actor = viewer.require()?;
    let (c, ctx) = load_comment(&state, &viewer, id).await?;
    if c.status != floor::STATUS_NORMAL || ctx.post.status != STATUS_NORMAL {
        return Err(ApiError(ErrorCode::NotFound));
    }
    let target = Target {
        kind: TARGET_COMMENT,
        id,
        forum_id: ctx.forum.id,
        author_id: c.author_id,
    };
    report(&state, actor.id, target, &req.reason).await
}
