//! 写操作 Origin 校验（防 CSRF，plan 第 5 节）与真实客户端 IP。

use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, Method, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use forum_core::ErrorCode;
use time::OffsetDateTime;
use tower_sessions::Session;

use crate::admin::is_admin_path;
use crate::error::{ApiError, ApiResult};
use crate::routes::{SESSION_LOGIN_AT, SESSION_TOUCHED_AT, SESSION_USER_ID};
use crate::state::AppState;

/// 会话最长有效期：登录满 90 天必须重新登录，不管期间多活跃。
pub const SESSION_MAX_AGE: time::Duration = time::Duration::days(90);
/// 已登录请求顺延闲置期限的最小间隔。
pub const SESSION_TOUCH_EVERY: time::Duration = time::Duration::hours(1);

/// 会话期限（plan 第 5 节）：闲置 30 天过期、最长 90 天。tower-sessions 只在会话数据变化时保存，闲置期限
/// 也只在保存时顺延，所以已登录的请求每小时最多「碰」一次会话（改 `touched_at`）：常用的人一直在线，
/// 不用的人 30 天后过期，每小时也最多写一次 Valkey。登录满 90 天的会话在这里清掉，之后的处理当作未登录。
pub async fn session_lifetime(session: Session, req: Request, next: Next) -> Response {
    match enforce_session_lifetime(&session).await {
        Ok(()) => next.run(req).await,
        Err(e) => e.into_response(),
    }
}

async fn enforce_session_lifetime(session: &Session) -> ApiResult<()> {
    if session.get::<i64>(SESSION_USER_ID).await?.is_none() {
        return Ok(());
    }
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let elapsed = |t: i64| time::Duration::seconds(now.saturating_sub(t));
    match session.get::<i64>(SESSION_LOGIN_AT).await? {
        Some(t) if elapsed(t) >= SESSION_MAX_AGE => {
            session.flush().await?;
            return Ok(());
        }
        Some(_) => {}
        // 加上最长期限之前登录的会话：从现在起算
        None => session.insert(SESSION_LOGIN_AT, now).await?,
    }
    let touched = session.get::<i64>(SESSION_TOUCHED_AT).await?;
    if touched.is_none_or(|t| elapsed(t) >= SESSION_TOUCH_EVERY) {
        session.insert(SESSION_TOUCHED_AT, now).await?;
    }
    Ok(())
}

/// `Origin` 在白名单内（论坛页面与钱包页面来源）；缺失为否。
pub fn origin_allowed(state: &AppState, headers: &HeaderMap) -> bool {
    headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|o| {
            state
                .allowed_origins
                .iter()
                .chain(&state.wallet.origins)
                .any(|a| a == o)
        })
}

/// 非安全方法必须带白名单内的 `Origin`（含钱包页面来源）；缺失也拒绝。
/// 后台接口另算：每个请求（含 GET）都必须来自后台页面（[`crate::admin`]），论坛与钱包来源一律拒绝。
pub async fn require_origin(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let ok = if is_admin_path(req.uri().path()) {
        *req.method() == Method::OPTIONS
            || req
                .headers()
                .get(header::ORIGIN)
                .is_some_and(|o| o.as_bytes() == state.admin.origin.as_bytes())
    } else {
        matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS)
            || origin_allowed(&state, req.headers())
    };
    if !ok {
        return ApiError(ErrorCode::OriginRejected).into_response();
    }
    next.run(req).await
}

/// 客户端 IP。只有连接来自本机（即 Caddy 反向代理）时才信任 `CF-Connecting-IP`：
/// Caddy 只接受带 Cloudflare 客户端证书的连接（plan 第 11 节 Authenticated Origin Pulls）。
#[derive(Debug, Clone, Copy)]
pub struct ClientIp(pub IpAddr);

impl<S: Send + Sync> FromRequestParts<S> for ClientIp {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let ConnectInfo(peer) = ConnectInfo::<SocketAddr>::from_request_parts(parts, state)
            .await
            .map_err(ApiError::internal)?;
        if peer.ip().is_loopback()
            && let Some(ip) = parts
                .headers
                .get("cf-connecting-ip")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse().ok())
        {
            return Ok(Self(ip));
        }
        Ok(Self(peer.ip()))
    }
}
