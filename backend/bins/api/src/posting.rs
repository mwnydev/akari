//! 发帖、回帖、评论、编辑共用的写入前检查：处罚、限流、新用户限制、内容过滤；以及写入后的缓存维护。

use std::net::IpAddr;
use std::time::Duration;

use forum_auth::Captcha;
use forum_content::Cleaned;
use forum_core::ErrorCode;
use forum_db::sanctions;
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
/// 楼层评论；新用户与发帖、回帖共用 [`NEW_USER_RATE`]。
pub const COMMENT_RATE: SlidingWindow =
    SlidingWindow::new("post-comment", 60, Duration::from_secs(600));
/// 点赞与取消赞合计。
pub const LIKE_RATE: SlidingWindow = SlidingWindow::new("post-like", 120, Duration::from_secs(600));
/// 举报楼层与评论合计。
pub const REPORT_RATE: SlidingWindow = SlidingWindow::new("report", 20, Duration::from_secs(3600));
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

/// 发帖、回帖、编辑前：被封号，或在该板块（及全站）被禁言时拒绝（ADR 0013）。
pub async fn check_sanctions(
    state: &AppState,
    user_id: i64,
    forum_id: Option<i32>,
) -> ApiResult<()> {
    let kinds = [sanctions::KIND_MUTE, sanctions::KIND_BAN];
    match sanctions::active_kind(&state.db, user_id, &kinds, forum_id).await? {
        Some(sanctions::KIND_BAN) => Err(ApiError(ErrorCode::AccountBanned)),
        Some(_) => Err(ApiError(ErrorCode::Muted)),
        None => Ok(()),
    }
}

/// 发私信、开始新交易（提交 / 修改报价卡片、接受报价；交易帖经 [`check_sanctions`]）前：被封号时拒绝
/// （禁言只限论坛发帖）。已有订单的操作、订单聊天与提币都不检查封号，资金不会因此卡在合约或钱包里。
pub async fn check_not_banned(state: &AppState, user_id: i64) -> ApiResult<()> {
    match sanctions::active_kind(&state.db, user_id, &[sanctions::KIND_BAN], None).await? {
        Some(_) => Err(ApiError(ErrorCode::AccountBanned)),
        None => Ok(()),
    }
}

/// 拍卖操作（发起拍卖、出价）前：被封号，或在该板块（及全站）被禁拍时拒绝（ADR 0013）。
/// 拍卖接口上线时在入口调用；已成交的拍卖订单与其他已有订单一样不受影响。
pub async fn check_can_auction(
    state: &AppState,
    user_id: i64,
    forum_id: Option<i32>,
) -> ApiResult<()> {
    let kinds = [sanctions::KIND_AUCTION_BAN, sanctions::KIND_BAN];
    match sanctions::active_kind(&state.db, user_id, &kinds, forum_id).await? {
        Some(sanctions::KIND_BAN) => Err(ApiError(ErrorCode::AccountBanned)),
        Some(_) => Err(ApiError(ErrorCode::AuctionBanned)),
        None => Ok(()),
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

/// 注册 24 小时内的新用户发主题、回帖、评论前先过人机验证（plan 第 11 节「刷帖、广告」：Turnstile + 新号限制）；
/// 组件的 `action` 为 `post`。没带令牌返回 `CAPTCHA_REQUIRED`（前端据此显示组件），令牌无效返回 `CAPTCHA_FAILED`；
/// Turnstile 不可用时拒绝。开发环境没配置密钥时一律通过。
pub async fn check_new_user_captcha(
    state: &AppState,
    actor: &Actor,
    token: Option<&str>,
    ip: IpAddr,
) -> ApiResult<()> {
    if !is_new_user(actor) {
        return Ok(());
    }
    let token = token.filter(|t| !t.is_empty());
    let passed = state
        .captcha
        .verify(token, ip, forum_auth::captcha::action::POST)
        .await
        .map_err(ApiError::internal)?;
    match (passed, token) {
        (true, _) => Ok(()),
        (false, None) => Err(ApiError(ErrorCode::CaptchaRequired)),
        (false, Some(_)) => Err(ApiError(ErrorCode::CaptchaFailed)),
    }
}

/// 过滤内容并检查新用户外链。在事务外调用（plan 第 8 节）。
pub fn clean(state: &AppState, actor: &Actor, html: &str, source: &Value) -> ApiResult<Cleaned> {
    let source_len = serde_json::to_vec(source)
        .map_err(ApiError::internal)?
        .len();
    if source_len > MAX_SOURCE_BYTES {
        return Err(ApiError(ErrorCode::ContentTooLong));
    }
    let image_hosts = state.image_hosts();
    let mut cleaned = forum_content::clean(html, &image_hosts)?;
    if is_new_user(actor)
        && forum_content::has_external_link(&cleaned.links, &state.internal_hosts, &image_hosts)
    {
        return Err(ApiError(ErrorCode::ExternalLinkForbidden));
    }
    cleaned.mentions.truncate(MAX_MENTIONS);
    Ok(cleaned)
}

/// 评论里 `@用户名` 对应的用户 ID（[`forum_core::floor::mention_names`]，不存在的用户名跳过）。
pub async fn comment_mentions(state: &AppState, body: &str) -> ApiResult<Vec<i64>> {
    let names = forum_core::floor::mention_names(body);
    Ok(forum_db::users::ids_by_names(&state.db, &names).await?)
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
