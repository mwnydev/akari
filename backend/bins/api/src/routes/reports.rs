//! 举报处理队列（plan 第 4 节「审核」、ADR 0013）：同一内容待处理的举报归成一件案件，按板块排队。
//!
//! - 谁能看：有处理举报权限的管理人员；版主只看自己负责的板块，管理员与站长看全部。内容作者本人看不到、
//!   也处理不了自己内容上的案件。举报人是谁只给管理员与站长看（版主只看到举报理由）。
//! - 怎么处理：驳回（不违规）、隐藏内容（楼层；1 楼隐藏主题）、删除内容、处罚作者。处理方式走与对应管理接口
//!   相同的权限、限速、异常检测与审计日志（`post.hide`、`user.mute` 等，可单独撤销），同一事务里结案并另写一条
//!   `report.resolve` 审计日志（一键撤销时重新打开案件）。
//! - 结案时每位举报人收到一条通知，只说「已处理 / 未发现违规」，不含处理方式与处罚内容。

use std::collections::{HashMap, HashSet};

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use forum_core::ErrorCode;
use forum_core::staff::{Permission, Scope};
use forum_db::floor::CommentRow;
use forum_db::posts;
use forum_db::reports::{self, Case, STATUS_ACTIONED, STATUS_DISMISSED, STATUS_OPEN, TARGET_POST};
use forum_db::topics::FLAG_HIDDEN;
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::OffsetDateTime;
use tower_sessions::Session;
use utoipa::{IntoParams, ToSchema};

use super::floor::{self, load_comment};
use super::moderation::{SanctionTerms, apply_sanction, authorize_sanction};
use super::posts::{Ctx, load, staff_delete, staff_hide};
use crate::access::Viewer;
use crate::admin::AdminSession;
use crate::cards;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::moderation::{
    self, Class, Moderator, TopicChange, TopicEffect, action, bump_page1, denied,
};
use crate::state::AppState;
use crate::views::UserCardView;

/// 每页案件数。
const PAGE: i64 = 30;
/// 每件案件列出最近几条举报。
const LATEST_REPORTS: i64 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueueStatus {
    #[default]
    Open,
    Resolved,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct QueueQuery {
    /// 待处理（默认）或已结案。
    #[param(inline)]
    pub status: Option<QueueStatus>,
    /// 游标：上一页的 `next_cursor`。
    pub cursor: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReportTargetKind {
    Post,
    Comment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReportStatus {
    Open,
    /// 未发现违规（驳回）。
    Dismissed,
    /// 已处理（隐藏、删除或处罚）。
    Actioned,
}

/// 被举报内容现在的状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContentState {
    Normal,
    Hidden,
    Deleted,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ReportTargetView {
    pub kind: ReportTargetKind,
    /// 楼层或评论的 ID。
    pub id: i64,
    /// 所在楼层（评论为它所在的楼层）。
    pub post_id: i64,
    pub topic_id: i64,
    pub topic_title: String,
    pub floor: i32,
    /// 内容现在所在的板块。
    pub forum_id: i32,
    pub author_id: i64,
    /// 纯文本前 200 字（评论为正文）。
    pub excerpt: String,
    pub state: ContentState,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ReportEntryView {
    /// 举报人：只给管理员与站长；版主为空。
    pub reporter_id: Option<i64>,
    pub reason: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ReportCaseView {
    pub id: i64,
    pub status: ReportStatus,
    /// 已处理时采取的措施：`hide` / `delete` / `sanction`。
    pub action: Option<String>,
    /// 举报次数（不同的人）。
    pub reports: i32,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub last_reported_at: OffsetDateTime,
    pub resolved_by: Option<i64>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub resolved_at: Option<OffsetDateTime>,
    pub target: ReportTargetView,
    /// 最近的几条举报，新的在前。
    pub latest: Vec<ReportEntryView>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ReportQueue {
    pub cases: Vec<ReportCaseView>,
    /// 内容作者、处理人（以及给管理员与站长的举报人）的名片。
    pub users: Vec<UserCardView>,
    /// 访问者能处理的待处理案件总数（入口上的数字）。
    pub open_count: i64,
    pub next_cursor: Option<i64>,
}

fn status_of(s: i16) -> ReportStatus {
    match s {
        STATUS_OPEN => ReportStatus::Open,
        STATUS_DISMISSED => ReportStatus::Dismissed,
        _ => ReportStatus::Actioned,
    }
}

/// 楼层的状态码（0 正常、3 隐藏，其余为删除）；评论没有隐藏状态，非 0 即删除。
fn content_state(status: i16) -> ContentState {
    match status {
        0 => ContentState::Normal,
        posts::STATUS_HIDDEN => ContentState::Hidden,
        _ => ContentState::Deleted,
    }
}

/// 访问者能处理举报的板块：版主为自己负责的，管理员与站长为全部；没有任何板块（含被暂停）时拒绝。
async fn report_boards(state: &AppState, viewer: &Viewer) -> ApiResult<(Vec<i32>, bool)> {
    let roles = viewer.staff(state).await?;
    let boards: Vec<i32> = forum_db::forums::list_all(&state.db)
        .await?
        .into_iter()
        .map(|f| f.id)
        .filter(|&f| roles.allows(Permission::Reports, Scope::Board(f)))
        .collect();
    if boards.is_empty() {
        return Err(denied(&roles));
    }
    Ok((boards, roles.allows(Permission::Reports, Scope::Site)))
}

/// 举报处理队列（新的在前）。版主只看自己负责的板块，管理员与站长看全部；不含访问者自己的内容。
#[utoipa::path(
    get, path = "/api/v1/moderation/reports", tag = "moderation", operation_id = "report_queue",
    params(QueueQuery),
    responses(
        (status = 200, body = ReportQueue),
        (status = 401, body = ErrorBody), (status = 403, body = ErrorBody)
    )
)]
pub async fn report_queue(
    State(state): State<AppState>,
    session: Session,
    Query(q): Query<QueueQuery>,
) -> ApiResult<Json<ReportQueue>> {
    let viewer = Viewer::load(&state, &session).await?;
    let actor = viewer.require()?;
    let (boards, site) = report_boards(&state, &viewer).await?;
    let open = q.status.unwrap_or_default() == QueueStatus::Open;
    let mut rows = reports::queue(&state.db, &boards, open, q.cursor, PAGE + 1, actor.id).await?;
    let more = rows.len() as i64 > PAGE;
    rows.truncate(PAGE as usize);
    let next_cursor = rows.last().map(|c| c.id).filter(|_| more);

    let (post_ids, comment_ids): (Vec<&Case>, Vec<&Case>) =
        rows.iter().partition(|c| c.target_kind == TARGET_POST);
    let post_ids: Vec<i64> = post_ids.iter().map(|c| c.target_id).collect();
    let comment_ids: Vec<i64> = comment_ids.iter().map(|c| c.target_id).collect();
    let mut post_targets: HashMap<i64, ReportTargetView> =
        reports::post_previews(&state.db, &post_ids)
            .await?
            .into_iter()
            .map(|p| {
                let view = ReportTargetView {
                    kind: ReportTargetKind::Post,
                    id: p.id,
                    post_id: p.id,
                    topic_id: p.topic_id,
                    topic_title: p.title,
                    floor: p.floor,
                    forum_id: p.forum_id,
                    author_id: p.author_id,
                    excerpt: p.excerpt,
                    state: content_state(p.status),
                };
                (p.id, view)
            })
            .collect();
    let mut comment_targets: HashMap<i64, ReportTargetView> =
        reports::comment_previews(&state.db, &comment_ids)
            .await?
            .into_iter()
            .map(|c| {
                let view = ReportTargetView {
                    kind: ReportTargetKind::Comment,
                    id: c.id,
                    post_id: c.post_id,
                    topic_id: c.topic_id,
                    topic_title: c.title,
                    floor: c.floor,
                    forum_id: c.forum_id,
                    author_id: c.author_id,
                    excerpt: c.excerpt,
                    state: content_state(c.status),
                };
                (c.id, view)
            })
            .collect();
    let case_ids: Vec<i64> = rows.iter().map(|c| c.id).collect();
    let mut latest: HashMap<i64, Vec<ReportEntryView>> = HashMap::new();
    for r in reports::latest_reports(&state.db, &case_ids, LATEST_REPORTS).await? {
        latest.entry(r.case_id).or_default().push(ReportEntryView {
            reporter_id: site.then_some(r.reporter_id),
            reason: r.reason,
            created_at: r.created_at,
        });
    }

    let allowed: HashSet<i32> = boards.iter().copied().collect();
    let mut users = Vec::new();
    let cases: Vec<ReportCaseView> = rows
        .into_iter()
        .filter_map(|c| {
            let target = if c.target_kind == TARGET_POST {
                post_targets.remove(&c.target_id)
            } else {
                comment_targets.remove(&c.target_id)
            }
            // 案件跟着主题移动（reports::follow_topic），这里再按内容现在的板块过滤一次
            .filter(|t| allowed.contains(&t.forum_id))?;
            let latest = latest.remove(&c.id).unwrap_or_default();
            users.push(target.author_id);
            users.extend(c.resolved_by);
            users.extend(latest.iter().filter_map(|r| r.reporter_id));
            Some(ReportCaseView {
                id: c.id,
                status: status_of(c.status),
                action: c.action,
                reports: c.reports,
                created_at: c.created_at,
                last_reported_at: c.last_reported_at,
                resolved_by: c.resolved_by,
                resolved_at: c.resolved_at,
                target,
                latest,
            })
        })
        .collect();
    Ok(Json(ReportQueue {
        cases,
        users: cards::load(&state, &users).await?,
        open_count: reports::open_count(&state.db, &boards, actor.id).await?,
        next_cursor,
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResolveAction {
    /// 未发现违规，驳回。
    Dismiss,
    /// 隐藏楼层（1 楼为隐藏主题）；评论不能隐藏，请删除。
    Hide,
    /// 删除楼层或评论（软删除，可恢复）。
    Delete,
    /// 处罚内容作者（`sanction` 必填）。
    Sanction,
}

impl ResolveAction {
    /// 写进案件的措施名（驳回为空）。
    fn name(self) -> Option<&'static str> {
        match self {
            Self::Dismiss => None,
            Self::Hide => Some("hide"),
            Self::Delete => Some("delete"),
            Self::Sanction => Some("sanction"),
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolveRequest {
    pub action: ResolveAction,
    /// 必填，1–500 字：写入处理方式与结案的审计日志；隐藏时显示给内容作者，处罚时通知被处罚人。
    pub reason: String,
    /// `sanction` 时必填。
    pub sanction: Option<SanctionTerms>,
}

/// 被举报的内容与它所在的楼层、主题。
enum Content {
    Post(Ctx),
    Comment(CommentRow, Ctx),
}

impl Content {
    fn ctx(&self) -> &Ctx {
        match self {
            Self::Post(ctx) | Self::Comment(_, ctx) => ctx,
        }
    }

    fn author_id(&self) -> i64 {
        match self {
            Self::Post(ctx) => ctx.post.author_id,
            Self::Comment(c, _) => c.author_id,
        }
    }
}

/// 提交后要做的缓存维护与推送。
#[derive(Default)]
struct Effects {
    page: Option<i64>,
    topic: Option<TopicEffect>,
    comment: Option<i32>,
}

/// 处理一件举报：按 `action` 驳回、隐藏、删除或处罚作者，并结案、通知举报人。需要内容现在所在板块的处理举报权限，
/// 以及处理方式本身的权限（隐藏 / 删除 / 处罚，版主限本板块、处罚时长有上限）；不能处理自己内容上的举报。
/// 内容已经是要求的状态（如已被删除）时只结案。
#[utoipa::path(
    post, path = "/api/v1/moderation/reports/{id}/resolve", tag = "moderation", operation_id = "resolve_report",
    params(("id" = i64, Path)),
    request_body = ResolveRequest,
    responses(
        (status = 204),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody), (status = 409, body = ErrorBody), (status = 429, body = ErrorBody)
    )
)]
pub async fn resolve_report(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
    Json(req): Json<ResolveRequest>,
) -> ApiResult<StatusCode> {
    let viewer = Viewer::load(&state, &session).await?;
    let actor = viewer.require()?;
    let reason = moderation::reason(&req.reason)?;
    let case = reports::case(&state.db, id)
        .await?
        .ok_or(ApiError(ErrorCode::NotFound))?;
    let content = if case.target_kind == TARGET_POST {
        Content::Post(load(&state, &viewer, case.target_id).await?)
    } else {
        let (c, ctx) = load_comment(&state, &viewer, case.target_id).await?;
        Content::Comment(c, ctx)
    };
    let ctx = content.ctx();
    let board = Scope::Board(ctx.topic.forum_id);
    let first_floor = matches!(content, Content::Post(_)) && ctx.post.floor == 1;
    let mut sanction = None;
    let m = match req.action {
        ResolveAction::Dismiss => {
            Moderator::authorize(&state, actor.id, Permission::Reports, board, Class::Content)
                .await?
        }
        ResolveAction::Hide => {
            let p = match &content {
                Content::Comment(..) => return Err(ApiError(ErrorCode::InvalidParameter)),
                Content::Post(_) if first_floor => Permission::TopicHide,
                Content::Post(_) => Permission::PostHide,
            };
            Moderator::authorize(&state, actor.id, p, board, Class::Delete).await?
        }
        ResolveAction::Delete => {
            Moderator::authorize(
                &state,
                actor.id,
                Permission::PostDelete,
                board,
                Class::Delete,
            )
            .await?
        }
        ResolveAction::Sanction => {
            let terms = req.sanction.ok_or(ApiError(ErrorCode::InvalidParameter))?;
            let (m, ends_at) =
                authorize_sanction(&state, actor.id, content.author_id(), terms).await?;
            sanction = Some((terms, ends_at));
            m
        }
    };
    m.require(Permission::Reports, board)?;
    if content.author_id() == actor.id {
        return Err(ApiError(ErrorCode::Forbidden));
    }
    if case.status != STATUS_OPEN {
        return Err(ApiError(ErrorCode::ReportResolved));
    }

    let mut tx = state.db.begin().await?;
    m.guard(&state, &mut tx).await?;
    let mut effects = Effects::default();
    match (req.action, &content) {
        (ResolveAction::Dismiss, _) => {}
        (ResolveAction::Hide, Content::Post(ctx)) if first_floor => {
            if ctx.topic.flags & FLAG_HIDDEN == 0 {
                let change = TopicChange::Flags {
                    set: FLAG_HIDDEN,
                    clear: 0,
                };
                let effect = moderation::change_topic(&mut tx, &ctx.topic, change)
                    .await?
                    .ok_or(ApiError(ErrorCode::NotFound))?;
                forum_db::audit::append(
                    &mut *tx,
                    actor.id,
                    action::TOPIC_HIDE,
                    Some(&format!("topic:{}", ctx.topic.id)),
                    Some(reason),
                    &effect.detail(),
                )
                .await?;
                effects.topic = Some(effect);
            }
        }
        (ResolveAction::Hide, Content::Post(ctx)) => {
            if ctx.post.status != posts::STATUS_HIDDEN {
                staff_hide(&mut tx, actor.id, ctx, reason).await?;
                effects.page = Some(ctx.topic.id);
            }
        }
        (ResolveAction::Delete, Content::Post(ctx)) => {
            if ctx.post.status == posts::STATUS_NORMAL || ctx.post.status == posts::STATUS_HIDDEN {
                staff_delete(&mut tx, actor.id, ctx, reason).await?;
                effects.page = Some(ctx.topic.id);
            }
        }
        (ResolveAction::Delete, Content::Comment(c, ctx)) => {
            if c.status == forum_db::floor::STATUS_NORMAL {
                let count =
                    floor::staff_delete_comment(&mut tx, actor.id, c, ctx.topic.id, reason).await?;
                effects.comment = Some(count);
            }
        }
        (ResolveAction::Sanction, _) => {
            if let Some((terms, ends_at)) = sanction {
                apply_sanction(
                    &mut tx,
                    actor.id,
                    content.author_id(),
                    terms,
                    ends_at,
                    reason,
                )
                .await?;
            }
        }
        (ResolveAction::Hide, Content::Comment(..)) => {
            return Err(ApiError(ErrorCode::InvalidParameter));
        }
    }
    let status = if req.action == ResolveAction::Dismiss {
        STATUS_DISMISSED
    } else {
        STATUS_ACTIONED
    };
    if !reports::resolve(
        &mut *tx,
        id,
        status,
        req.action.name(),
        actor.id,
        ctx.topic.id,
        ctx.post.floor,
    )
    .await?
    {
        return Err(ApiError(ErrorCode::ReportResolved));
    }
    forum_db::audit::append(
        &mut *tx,
        actor.id,
        action::REPORT_RESOLVE,
        Some(&format!("report:{id}")),
        Some(reason),
        &json!({
            "case_id": id, "status": status, "action": req.action.name(),
            "target_kind": case.target_kind, "target_id": case.target_id, "reports": case.reports,
        }),
    )
    .await?;
    tx.commit().await?;

    if let Some(t) = effects.topic {
        t.apply(&state).await;
    }
    if let Some(topic_id) = effects.page.filter(|_| ctx.post.floor <= posts::PAGE_SIZE) {
        bump_page1(&state, topic_id).await;
    }
    if let (Some(count), Content::Comment(c, ctx)) = (effects.comment, &content) {
        floor::push_comment_removed(&state, &ctx.topic, c, count).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ForumReportCount {
    pub forum_id: i32,
    pub open: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AdminReportSummary {
    /// 全站待处理的举报案件数。
    pub open: i64,
    /// 有待处理案件的板块。
    pub by_forum: Vec<ForumReportCount>,
}

/// 后台概览：各板块待处理的举报案件数（处理在论坛的「举报处理」页）。
#[utoipa::path(
    get, path = "/api/v1/admin/reports", tag = "admin", operation_id = "admin_report_summary",
    responses((status = 200, body = AdminReportSummary), (status = 401, body = ErrorBody))
)]
pub async fn admin_report_summary(
    State(state): State<AppState>,
    // 后台会话本身就确认了是站长
    _admin: AdminSession,
) -> ApiResult<Json<AdminReportSummary>> {
    let by_forum: Vec<ForumReportCount> = reports::open_by_forum(&state.db)
        .await?
        .into_iter()
        .map(|(forum_id, open)| ForumReportCount { forum_id, open })
        .collect();
    Ok(Json(AdminReportSummary {
        open: by_forum.iter().map(|f| f.open).sum(),
        by_forum,
    }))
}
