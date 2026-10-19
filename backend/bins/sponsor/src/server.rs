//! 本机 HTTP 接口：只给 forum-api 调用。
//!
//! - `POST /rpc`：JSON-RPC 2.0（单个请求，不支持批量）。必须带 `Authorization: Bearer <SPONSOR_API_TOKEN>` 与
//!   forum-api 校验过会话后填入的 `x-forum-user-id`。监听 127.0.0.1，令牌防的是本机其他进程冒充 forum-api。
//! - `GET /healthz`：代发器是否健康。

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::routing::{get, post};
use serde_json::{Value, json};

use crate::bundler::Bundler;
use crate::jsonrpc::{self, INVALID_REQUEST, Request, RpcError};

pub const USER_HEADER: &str = "x-forum-user-id";
const BODY_LIMIT: usize = 128 * 1024;

#[derive(Clone)]
struct AppState {
    bundler: Arc<Bundler>,
    token: Arc<str>,
}

/// 逐字节比较，耗时与内容无关。
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn authorized(headers: &HeaderMap, token: &str) -> bool {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|t| constant_time_eq(t.as_bytes(), token.as_bytes()))
}

async fn rpc(
    State(s): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, StatusCode> {
    if !authorized(&headers, &s.token) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let user_id: i64 = headers
        .get(USER_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .ok_or(StatusCode::BAD_REQUEST)?;
    let req: Request = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => {
            let err = RpcError::new(INVALID_REQUEST, "expected a single JSON-RPC request object");
            return Ok(Json(jsonrpc::response(Value::Null, Err(err))));
        }
    };
    Ok(Json(s.bundler.handle(user_id, req).await))
}

async fn healthz(State(s): State<AppState>) -> Json<Value> {
    Json(json!({
        "relayer": s.bundler.relayer.healthy(),
        "fallback": s.bundler.fallback.is_some(),
    }))
}

pub fn router(bundler: Arc<Bundler>, token: &str) -> Router {
    Router::new()
        .route("/rpc", post(rpc))
        .route("/healthz", get(healthz))
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .with_state(AppState {
            bundler,
            token: Arc::from(token),
        })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn token_comparison() {
        let mut h = HeaderMap::new();
        assert!(!authorized(&h, "secret"));
        h.insert(header::AUTHORIZATION, "Bearer secret".parse().unwrap());
        assert!(authorized(&h, "secret"));
        assert!(!authorized(&h, "secreT"));
        assert!(!authorized(&h, "secret2"));
    }
}
