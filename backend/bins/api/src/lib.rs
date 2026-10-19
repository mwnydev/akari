//! forum-api：论坛 HTTP API 与 WebSocket。

pub mod config;
pub mod error;
pub mod middleware;
pub mod routes;
pub mod state;

use std::time::Duration;

use axum::http::StatusCode;
use axum::{Json, Router, middleware::from_fn_with_state, routing::get};
use tower_http::{timeout::TimeoutLayer, trace::TraceLayer};
use tower_sessions::cookie::{Key, SameSite};
use tower_sessions::{Expiry, SessionManagerLayer};
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

pub use state::AppState;

/// 会话 Cookie 名。
pub const SESSION_COOKIE: &str = "sid";
/// 普通会话闲置 30 天过期（plan 第 5 节）。
const SESSION_IDLE_DAYS: i64 = 30;

#[derive(OpenApi)]
#[openapi(info(title = "akari forum API", version = "v1"))]
struct ApiDoc;

/// 生成 OpenAPI 文档与路由。
pub fn router() -> (Router<AppState>, utoipa::openapi::OpenApi) {
    OpenApiRouter::with_openapi(ApiDoc::openapi())
        .routes(routes!(routes::health::healthz))
        .routes(routes!(routes::auth::start))
        .routes(routes!(routes::auth::verify))
        .routes(routes!(routes::auth::logout))
        .routes(routes!(routes::me::me))
        .split_for_parts()
}

/// 组装完整应用。`session_key` 为 64 字节签名密钥。
pub fn app(state: AppState, session_key: &[u8]) -> Router {
    let (api, doc) = router();
    let sessions = SessionManagerLayer::new(forum_kv::ValkeySessionStore::new(state.kv.clone()))
        .with_name(SESSION_COOKIE)
        .with_http_only(true)
        .with_secure(state.cookie_secure)
        .with_same_site(SameSite::Lax)
        .with_expiry(Expiry::OnInactivity(time::Duration::days(
            SESSION_IDLE_DAYS,
        )))
        .with_signed(Key::from(session_key));
    api.route(
        "/api/v1/openapi.json",
        get(move || async move { Json(doc) }),
    )
    .layer(from_fn_with_state(
        state.clone(),
        middleware::require_origin,
    ))
    .layer(sessions)
    .layer(TraceLayer::new_for_http())
    .layer(TimeoutLayer::with_status_code(
        StatusCode::REQUEST_TIMEOUT,
        Duration::from_secs(10),
    ))
    .with_state(state)
}
