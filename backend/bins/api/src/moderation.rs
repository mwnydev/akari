//! 管理操作的共用流程（ADR 0013）：权限（[`forum_core::staff`]）→ 限速 → 事务内异常检测 → 写审计日志；
//! 以及按审计日志撤销管理操作（站长一键撤销）。论坛内所有管理接口都经 [`Moderator`]。
//!
//! 异常检测按审计日志计数（数据库是唯一事实来源，Valkey 只做限速）：同一操作者的管理事务先拿事务级锁，
//! 窗口内删帖 / 处罚已达阈值时本次操作不执行，暂停该账号的管理权限并推送管理员群，待站长在后台恢复。

use std::time::Duration;

use forum_core::ErrorCode;
use forum_core::staff::{ModerationSettings, Permission, Scope, StaffRoles};
use forum_db::topics::{self, FLAG_FEATURED, Topic};
use forum_db::{Postgres, Transaction, audit, outbox, reports, sanctions};
use forum_db::{floor, posts};
use forum_kv::SlidingWindow;
use forum_kv::forum_cache;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;
use utoipa::ToSchema;

use crate::cards;
use crate::error::{ApiError, ApiResult};
use crate::guest;
use crate::posting::{after_commit, micros};
use crate::state::AppState;

/// 审计日志里的操作名（主题标志位的操作名见 `routes::topics::ModerationAction`）。
pub mod action {
    pub const TOPIC_HIDE: &str = "topic.hide";
    pub const POST_EDIT: &str = "post.edit";
    pub const POST_DELETE: &str = "post.delete";
    pub const POST_RESTORE: &str = "post.restore";
    pub const POST_HIDE: &str = "post.hide";
    pub const POST_UNHIDE: &str = "post.unhide";
    pub const COMMENT_DELETE: &str = "comment.delete";
    pub const COMMENT_RESTORE: &str = "comment.restore";
    pub const MUTE: &str = "user.mute";
    pub const AUCTION_BAN: &str = "user.auction_ban";
    pub const BAN: &str = "user.ban";
    pub const DEVICE_BAN: &str = "user.device_ban";
    pub const IP_BAN: &str = "ip.ban";
    pub const SANCTION_LIFT: &str = "sanction.lift";
    /// 举报结案（处理方式本身另有一条日志，如 `post.hide`）；撤销即重新打开案件。
    pub const REPORT_RESOLVE: &str = "report.resolve";
}

/// 限速与异常检测的操作类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// 置顶、精华、锁定、移动、编辑、恢复。
    Content,
    /// 删帖、删评论、隐藏主题与楼层。
    Delete,
    /// 处罚与解除处罚。
    Sanction,
}

impl Class {
    fn rule(self, s: &ModerationSettings) -> SlidingWindow {
        let (name, limit) = match self {
            Self::Content => ("staff-content", s.rate_content),
            Self::Delete => ("staff-delete", s.rate_delete),
            Self::Sanction => ("staff-sanction", s.rate_sanction),
        };
        SlidingWindow::new(name, limit, Duration::from_secs(s.rate_window_secs.into()))
    }

    /// 计入异常检测的操作与阈值；内容类不计。
    fn anomaly(self, s: &ModerationSettings) -> Option<(&'static [&'static str], u32)> {
        match self {
            Self::Content => None,
            Self::Delete => Some((
                &[
                    action::POST_DELETE,
                    action::POST_HIDE,
                    action::COMMENT_DELETE,
                    action::TOPIC_HIDE,
                ],
                s.anomaly_deletes,
            )),
            Self::Sanction => Some((
                &[
                    action::MUTE,
                    action::AUCTION_BAN,
                    action::BAN,
                    action::DEVICE_BAN,
                    action::IP_BAN,
                ],
                s.anomaly_sanctions,
            )),
        }
    }
}

/// 已核对权限并通过限速的管理操作者。
#[derive(Debug, Clone)]
pub struct Moderator {
    pub id: i64,
    pub roles: StaffRoles,
    settings: ModerationSettings,
    class: Class,
}

/// 没有权限：被暂停的管理人员返回 `STAFF_SUSPENDED`，其他人 `FORBIDDEN`。
pub fn denied(roles: &StaffRoles) -> ApiError {
    if roles.suspended && roles.is_staff() {
        ApiError(ErrorCode::StaffSuspended)
    } else {
        ApiError(ErrorCode::Forbidden)
    }
}

impl Moderator {
    /// 核对 `actor_id` 能否在 `scope` 内做 `p`，再按 `class` 限速（Valkey 滑动窗口，每个操作者每类一个窗口）。
    pub async fn authorize(
        state: &AppState,
        actor_id: i64,
        p: Permission,
        scope: Scope,
        class: Class,
    ) -> ApiResult<Self> {
        let roles = forum_db::staff::roles(&state.db, actor_id).await?;
        if !roles.allows(p, scope) {
            return Err(denied(&roles));
        }
        let settings = forum_db::settings::moderation(&state.db).await?;
        crate::posting::limit(state, &class.rule(&settings), actor_id).await?;
        Ok(Self {
            id: actor_id,
            roles,
            settings,
            class,
        })
    }

    /// 同一操作里的另一项权限（如移动主题时的目标板块）。
    pub fn require(&self, p: Permission, scope: Scope) -> ApiResult<()> {
        if self.roles.allows(p, scope) {
            Ok(())
        } else {
            Err(denied(&self.roles))
        }
    }

    /// 管理事务开始后、写入之前调用：拿操作者锁并做异常检测。超过阈值时暂停管理权限、推送管理员群，
    /// 返回 `STAFF_SUSPENDED`（调用方的事务随之回滚，本次操作不执行）。
    pub async fn guard(
        &self,
        state: &AppState,
        tx: &mut Transaction<'_, Postgres>,
    ) -> ApiResult<()> {
        audit::lock_actor(&mut **tx, self.id).await?;
        let Some((actions, limit)) = self.class.anomaly(&self.settings) else {
            return Ok(());
        };
        let window = i64::from(self.settings.anomaly_window_secs);
        let count = audit::count_recent(&mut **tx, self.id, actions, window).await?;
        if count < i64::from(limit) {
            return Ok(());
        }
        let detail =
            json!({ "actions": actions, "count": count, "limit": limit, "window_secs": window });
        let mut alert = state.db.begin().await?;
        if forum_db::staff::suspend(&mut *alert, self.id, "anomaly", &detail)
            .await?
            .is_some()
        {
            let text = format!(
                "管理操作异常：账号 #{} 在 {} 分钟内已有 {count} 次删帖 / 处罚类操作（阈值 {limit}），\
                 管理权限已自动暂停，请站长在后台核实后恢复或撤销其操作。",
                self.id,
                window / 60
            );
            outbox::staff_alert(&mut *alert, "staff.anomaly", &text).await?;
        }
        alert.commit().await?;
        tracing::warn!(
            actor = self.id,
            count,
            limit,
            "management permissions suspended by anomaly detection"
        );
        Err(ApiError(ErrorCode::StaffSuspended))
    }
}

/// 处罚理由与管理操作理由：1–500 字。
pub fn reason(raw: &str) -> ApiResult<&str> {
    let r = raw.trim();
    if r.is_empty() || r.chars().count() > 500 {
        return Err(ApiError(ErrorCode::InvalidParameter));
    }
    Ok(r)
}

/// 改主题：标志位或板块。
#[derive(Debug, Clone, Copy)]
pub enum TopicChange {
    Flags { set: i16, clear: i16 },
    Move(i32),
}

/// 主题改动后要做的缓存维护与名片失效（事务提交后执行）。
#[derive(Debug, Clone, Copy)]
pub struct TopicEffect {
    topic_id: i64,
    author_id: i64,
    last_post_at: OffsetDateTime,
    pub old_forum: i32,
    pub new_forum: i32,
    pub old_flags: i16,
    pub new_flags: i16,
    level_changed: bool,
}

impl TopicEffect {
    /// 审计日志里记录的改动（撤销时据此还原）。
    pub fn detail(&self) -> Value {
        if self.old_forum == self.new_forum {
            json!({ "old_flags": self.old_flags, "new_flags": self.new_flags })
        } else {
            json!({ "from": self.old_forum, "to": self.new_forum })
        }
    }

    /// 维护有序集合成员关系、置顶列表与第 1 页缓存；精华经验让等级变化时刷新名片。
    pub async fn apply(&self, state: &AppState) {
        if self.level_changed {
            cards::invalidate(state, &[self.author_id]).await;
        }
        let was_listed = self.old_flags & topics::FLAGS_NOT_LISTED == 0;
        let is_listed = self.new_flags & topics::FLAGS_NOT_LISTED == 0;
        let moved = self.old_forum != self.new_forum;
        if was_listed && (!is_listed || moved) {
            after_commit(
                "board_remove",
                state.cache.board_remove(self.old_forum, self.topic_id),
            )
            .await;
        }
        if is_listed && (!was_listed || moved) {
            after_commit(
                "board_touch",
                state
                    .cache
                    .board_touch(self.new_forum, self.topic_id, micros(self.last_post_at)),
            )
            .await;
        }
        after_commit(
            "bump pinned",
            state.cache.bump(&forum_cache::pinned(self.old_forum)),
        )
        .await;
        if moved {
            after_commit(
                "bump pinned",
                state.cache.bump(&forum_cache::pinned(self.new_forum)),
            )
            .await;
        }
        bump_page1(state, self.topic_id).await;
        let forums = if moved {
            vec![self.old_forum, self.new_forum]
        } else {
            vec![self.old_forum]
        };
        guest::listing_changed(state, self.topic_id, &forums).await;
    }
}

/// 主题内容变化：第 1 页缓存与游客缓存里该主题的各页。
pub async fn bump_page1(state: &AppState, topic_id: i64) {
    after_commit(
        "bump page1",
        state.cache.bump(&forum_cache::topic_page1(topic_id)),
    )
    .await;
    guest::topic_changed(state, topic_id).await;
}

/// 在事务内改主题；主题已不存在返回 `None`。精华经验（plan 第 7 节）与标志位同一事务。
pub async fn change_topic(
    tx: &mut Transaction<'_, Postgres>,
    topic: &Topic,
    change: TopicChange,
) -> ApiResult<Option<TopicEffect>> {
    let (old_forum, new_forum, old_flags, new_flags) = match change {
        TopicChange::Move(to) => match topics::move_to(&mut **tx, topic.id, to).await? {
            Some(from) => {
                reports::follow_topic(&mut **tx, topic.id, to).await?;
                (from, to, topic.flags, topic.flags)
            }
            None => return Ok(None),
        },
        TopicChange::Flags { set, clear } => {
            match topics::update_flags(&mut **tx, topic.id, set, clear).await? {
                Some((old, new, forum)) => (forum, forum, old, new),
                None => return Ok(None),
            }
        }
    };
    let featured_now = new_flags & FLAG_FEATURED != 0;
    let level_changed = if featured_now != (old_flags & FLAG_FEATURED != 0) {
        let day = OffsetDateTime::now_utc().date();
        forum_db::exp::set_featured(tx, topic.author_id, topic.id, featured_now, day).await?
            == Some(true)
    } else {
        false
    };
    Ok(Some(TopicEffect {
        topic_id: topic.id,
        author_id: topic.author_id,
        last_post_at: topic.last_post_at,
        old_forum,
        new_forum,
        old_flags,
        new_flags,
        level_changed,
    }))
}

/// 撤销一条管理操作的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RevertStatus {
    /// 已撤销（预览时表示可以撤销）。
    Reverted,
    /// 之前已经撤销过。
    AlreadyReverted,
    /// 之后又有改动（或处罚已到期 / 已解除），不再覆盖。
    Superseded,
    /// 这类操作不在一键撤销范围内（如后台任免，用反向操作处理）。
    NotRevertible,
}

/// 撤销后要做的缓存维护。
#[derive(Debug, Default)]
pub struct RevertEffects {
    topics: Vec<TopicEffect>,
    pages: Vec<i64>,
}

impl RevertEffects {
    pub async fn apply(&self, state: &AppState) {
        for t in &self.topics {
            t.apply(state).await;
        }
        for p in &self.pages {
            bump_page1(state, *p).await;
        }
    }
}

#[derive(Deserialize)]
struct FlagDetail {
    old_flags: i16,
    new_flags: i16,
}

#[derive(Deserialize)]
struct MoveDetail {
    from: i32,
    to: i32,
}

#[derive(Deserialize)]
struct PostDetail {
    topic_id: i64,
    edit_id: Option<i64>,
    /// 取消隐藏前的隐藏理由（`post.unhide`）。
    hidden_reason: Option<String>,
}

#[derive(Deserialize)]
struct ReportDetail {
    case_id: i64,
}

#[derive(Deserialize)]
struct SanctionsDetail {
    sanction_ids: Vec<i64>,
}

#[derive(Deserialize)]
struct LiftDetail {
    sanction_id: i64,
}

fn target_id(entry: &audit::Entry, prefix: &str) -> Option<i64> {
    entry.target.as_deref()?.strip_prefix(prefix)?.parse().ok()
}

fn parse<T: serde::de::DeserializeOwned>(detail: &Value) -> Option<T> {
    serde_json::from_value(detail.clone()).ok()
}

/// 在事务内撤销一条管理操作（不写撤销日志，由调用方写）。只还原仍是该操作结果的状态：
/// 之后被别人改过的（如又被取消隐藏、又被编辑）算 [`RevertStatus::Superseded`]，不覆盖后来的改动。
pub async fn revert(
    state: &AppState,
    tx: &mut Transaction<'_, Postgres>,
    by: i64,
    entry: &audit::Entry,
    effects: &mut RevertEffects,
) -> ApiResult<RevertStatus> {
    use RevertStatus::{NotRevertible, Reverted, Superseded};
    if entry.reverted {
        return Ok(RevertStatus::AlreadyReverted);
    }
    let done = |ok: bool| if ok { Reverted } else { Superseded };
    let a = entry.action.as_str();
    if a.starts_with("topic.") {
        let Some(id) = target_id(entry, "topic:") else {
            return Ok(NotRevertible);
        };
        let Some(topic) = topics::by_id(&mut **tx, id).await? else {
            return Ok(Superseded);
        };
        let change = if let Some(d) = parse::<MoveDetail>(&entry.detail) {
            if topic.forum_id != d.to {
                return Ok(Superseded);
            }
            TopicChange::Move(d.from)
        } else if let Some(d) = parse::<FlagDetail>(&entry.detail) {
            let changed = d.old_flags ^ d.new_flags;
            if changed == 0 || topic.flags & changed != d.new_flags & changed {
                return Ok(Superseded);
            }
            TopicChange::Flags {
                set: d.old_flags & changed,
                clear: d.new_flags & changed,
            }
        } else {
            return Ok(NotRevertible);
        };
        return Ok(match change_topic(tx, &topic, change).await? {
            Some(e) => {
                effects.topics.push(e);
                Reverted
            }
            None => Superseded,
        });
    }
    match a {
        action::POST_DELETE
        | action::POST_RESTORE
        | action::POST_EDIT
        | action::POST_HIDE
        | action::POST_UNHIDE => {
            let (Some(id), Some(d)) = (
                target_id(entry, "post:"),
                parse::<PostDetail>(&entry.detail),
            ) else {
                return Ok(NotRevertible);
            };
            let ok = match a {
                action::POST_DELETE => posts::restore(&mut **tx, id).await?,
                action::POST_RESTORE => {
                    posts::soft_delete(&mut **tx, id, posts::STATUS_DELETED_BY_STAFF).await?
                }
                action::POST_HIDE => posts::unhide(&mut **tx, id).await?,
                // 重新隐藏，理由用取消隐藏时记下的原隐藏理由
                action::POST_UNHIDE => {
                    let Some(reason) = d.hidden_reason.as_deref() else {
                        return Ok(NotRevertible);
                    };
                    posts::hide(&mut **tx, id, reason, by).await?
                }
                _ => match d.edit_id {
                    Some(edit_id) => restore_edit(state, tx, by, id, edit_id).await?,
                    // 本轮之前的编辑日志没有记录编辑历史 ID
                    None => return Ok(NotRevertible),
                },
            };
            if ok {
                effects.pages.push(d.topic_id);
            }
            Ok(done(ok))
        }
        // 评论不在第 1 页缓存里，但在游客缓存的主题页里；打开着的页面要刷新才看到
        action::COMMENT_DELETE | action::COMMENT_RESTORE => {
            let Some(id) = target_id(entry, "comment:") else {
                return Ok(NotRevertible);
            };
            let changed = if a == action::COMMENT_DELETE {
                floor::restore(&mut **tx, id).await?
            } else {
                floor::soft_delete(&mut **tx, id, floor::STATUS_DELETED_BY_STAFF).await?
            };
            if changed.is_some()
                && let Some(c) = floor::comment(&mut **tx, id).await?
                && let Some(p) = posts::by_id(&mut **tx, c.post_id).await?
            {
                effects.pages.push(p.topic_id);
            }
            Ok(done(changed.is_some()))
        }
        action::MUTE | action::AUCTION_BAN | action::BAN | action::DEVICE_BAN | action::IP_BAN => {
            let Some(d) = parse::<SanctionsDetail>(&entry.detail) else {
                return Ok(NotRevertible);
            };
            Ok(done(
                !sanctions::revoke(&mut **tx, &d.sanction_ids, by)
                    .await?
                    .is_empty(),
            ))
        }
        action::REPORT_RESOLVE => match parse::<ReportDetail>(&entry.detail) {
            Some(d) => Ok(done(reports::reopen(&mut **tx, d.case_id).await?)),
            None => Ok(NotRevertible),
        },
        action::SANCTION_LIFT => match parse::<LiftDetail>(&entry.detail) {
            Some(d) => Ok(done(sanctions::unrevoke(&mut **tx, d.sanction_id).await?)),
            None => Ok(NotRevertible),
        },
        _ => Ok(NotRevertible),
    }
}

/// 把楼层恢复成某条编辑历史保存的版本（作为一次新的编辑，原有历史保留）；之后又被编辑过则不恢复。
async fn restore_edit(
    state: &AppState,
    tx: &mut Transaction<'_, Postgres>,
    by: i64,
    post_id: i64,
    edit_id: i64,
) -> ApiResult<bool> {
    let Some(snap) = posts::snapshot(&mut **tx, edit_id).await? else {
        return Ok(false);
    };
    if snap.superseded || snap.post_id != post_id {
        return Ok(false);
    }
    let Some(post) = posts::by_id(&mut **tx, post_id).await? else {
        return Ok(false);
    };
    let topic = topics::by_id(&mut **tx, post.topic_id).await?;
    // 历史里存的是当时已过滤的 HTML；再过滤一次得到纯文本。
    let cleaned = forum_content::clean(&snap.html, &state.image_hosts())?;
    let old_title = snap
        .title
        .as_ref()
        .and(topic.as_ref())
        .map(|t| t.title.as_str());
    let edited = posts::edit(
        tx,
        posts::Edit {
            post_id,
            editor_id: by,
            html: &cleaned.html,
            plain_text: &cleaned.plain_text,
            source: &snap.source,
            old_title,
        },
    )
    .await?;
    if edited.is_none() {
        return Ok(false);
    }
    if let (Some(title), Some(t)) = (&snap.title, &topic) {
        topics::set_title(&mut **tx, t.id, title).await?;
    }
    Ok(true)
}
