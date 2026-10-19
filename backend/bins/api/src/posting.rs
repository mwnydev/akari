//! 发帖、回帖、编辑共用的写入前检查：限流、新用户限制、内容过滤；以及写入后的缓存维护。

use std::time::Duration;

use forum_content::Cleaned;
use forum_core::ErrorCode;
use forum_db::users::Actor;
use forum_kv::SlidingWindow;
use serde_json::Value;
use time::OffsetDateTime;

use crate::access::is_new_user;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

// 初始值，按压测结果调整（plan 第 9 节「限流」）。
pub const TOPIC_RATE: SlidingWindow =
    SlidingWindow::new("post-topic", 10, Duration::from_secs(3600));
pub const REPLY_RATE: SlidingWindow =
    SlidingWindow::new("post-reply", 60, Duration::from_secs(600));
pub const EDIT_RATE: SlidingWindow = SlidingWindow::new("post-edit", 30, Duration::from_secs(600));
/// 注册 24 小时内：发主题与回帖合计每小时最多 5 条（plan 第 9 节）。
pub const NEW_USER_RATE: SlidingWindow =
    SlidingWindow::new("post-new-user", 5, Duration::from_secs(3600));

/// 编辑器原稿（Tiptap JSON）最大字节数。
pub const MAX_SOURCE_BYTES: usize = 256 * 1024;
/// 一条帖子最多通知多少个被 @ 的人。
pub const MAX_MENTIONS: usize = 20;

pub async fn limit(state: &AppState, rule: &SlidingWindow, user_id: i64) -> ApiResult<()> {
    let id = format!("{}{user_id}", state.cache.ns());
    if rule.hit(&state.kv, &id).await? {
        Ok(())
    } else {
        Err(ApiError(ErrorCode::RateLimited))
    }
}

/// 新发内容的限流：新用户统一按 [`NEW_USER_RATE`]，其他人按各自的规则。
pub async fn limit_new_content(
    state: &AppState,
    actor: &Actor,
    rule: &SlidingWindow,
) -> ApiResult<()> {
    let rule = if is_new_user(actor) {
        &NEW_USER_RATE
    } else {
        rule
    };
    limit(state, rule, actor.id).await
}

/// 过滤内容并检查新用户外链。在事务外调用（plan 第 8 节）。
pub fn clean(state: &AppState, actor: &Actor, html: &str, source: &Value) -> ApiResult<Cleaned> {
    let source_len = serde_json::to_vec(source)
        .map_err(ApiError::internal)?
        .len();
    if source_len > MAX_SOURCE_BYTES {
        return Err(ApiError(ErrorCode::ContentTooLong));
    }
    let mut cleaned = forum_content::clean(html, &state.image_hosts())?;
    if is_new_user(actor) && forum_content::has_external_link(&cleaned.links, &state.internal_hosts)
    {
        return Err(ApiError(ErrorCode::ExternalLinkForbidden));
    }
    cleaned.mentions.truncate(MAX_MENTIONS);
    Ok(cleaned)
}

/// Valkey 有序集合的分数：`last_post_at` 的微秒时间戳。
pub fn micros(t: OffsetDateTime) -> i64 {
    i64::try_from(t.unix_timestamp_nanos() / 1000).unwrap_or(i64::MAX)
}

/// 写库成功后的缓存维护失败不影响请求结果（数据库是唯一事实来源），只记日志。
pub async fn after_commit<F>(what: &'static str, fut: F)
where
    F: std::future::Future<Output = Result<(), forum_kv::KvError>>,
{
    if let Err(e) = fut.await {
        tracing::error!(error = %e, what, "cache maintenance failed");
    }
}
