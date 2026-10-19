//! 写操作 Origin 校验（防 CSRF，plan 第 5 节）与真实客户端 IP。

use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{Method, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use forum_core::ErrorCode;

use crate::error::ApiError;
use crate::state::AppState;

/// 非安全方法必须带白名单内的 `Origin`；缺失也拒绝。
pub async fn require_origin(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let safe = matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if !safe {
        let ok = req
            .headers()
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|o| state.allowed_origins.iter().any(|a| a == o));
        if !ok {
            return ApiError(ErrorCode::OriginRejected).into_response();
        }
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
