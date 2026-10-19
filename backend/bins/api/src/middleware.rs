//! 写操作 Origin 校验（防 CSRF，plan 第 5 节）与真实客户端 IP。

use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, Method, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use forum_core::ErrorCode;

use crate::admin::is_admin_path;
use crate::error::ApiError;
use crate::state::AppState;

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
