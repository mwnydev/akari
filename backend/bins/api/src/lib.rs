//! forum-api：论坛 HTTP API 与 WebSocket（`GET /api/v1/ws`，协议见 [`routes::ws`]）。

pub mod access;
pub mod admin;
pub mod arbitration;
pub mod cards;
pub mod config;
pub mod error;
pub mod middleware;
pub mod moderation;
pub mod posting;
pub mod realtime;
pub mod routes;
pub mod state;
pub mod views;
pub mod wallet;

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;
use axum::{Json, Router, middleware::from_fn_with_state, routing::get};
use tower::{Layer, ServiceExt};
use tower_http::cors::{AllowOrigin, CorsLayer};
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
        .routes(routes!(routes::me::set_username))
        .routes(routes!(routes::me::set_avatar))
        .routes(routes!(routes::users::user_profile))
        .routes(routes!(routes::users::user_posts))
        .routes(routes!(routes::users::search_users))
        .routes(routes!(routes::users::user_cards))
        .routes(routes!(routes::uploads::create_upload))
        .routes(routes!(routes::uploads::complete_upload))
        .routes(routes!(routes::uploads::get_upload))
        .routes(routes!(routes::forums::list))
        .routes(routes!(routes::forums::board, routes::forums::create_topic))
        .routes(routes!(routes::topics::latest_topics))
        .routes(routes!(routes::topics::get))
        .routes(routes!(routes::topics::reply))
        .routes(routes!(routes::topics::moderate))
        .routes(routes!(routes::posts::edit, routes::posts::delete))
        .routes(routes!(routes::posts::history))
        .routes(routes!(routes::posts::source))
        .routes(routes!(routes::posts::restore))
        .routes(routes!(
            routes::moderation::user_sanctions,
            routes::moderation::sanction_user
        ))
        .routes(routes!(routes::moderation::ban_ip))
        .routes(routes!(routes::moderation::lift))
        .routes(routes!(routes::wallet::overview))
        .routes(routes!(routes::wallet::reverify_start))
        .routes(routes!(routes::wallet::reverify_verify))
        .routes(routes!(routes::wallet::register_start))
        .routes(routes!(routes::wallet::register_finish))
        .routes(routes!(routes::wallet::assert_start))
        .routes(routes!(routes::wallet::assert_finish))
        .routes(routes!(routes::wallet::create_account))
        .routes(routes!(routes::aa::rpc))
        .routes(routes!(routes::chain::chain_rpc))
        .routes(routes!(routes::chain::balances))
        .routes(routes!(
            routes::withdraw::list_withdraw_addresses,
            routes::withdraw::add_withdraw_address
        ))
        .routes(routes!(routes::withdraw::revoke_withdraw_address))
        .routes(routes!(routes::trade::params))
        .routes(routes!(routes::trade::create_listing))
        .routes(routes!(routes::trade::get_listing))
        .routes(routes!(routes::trade::listing_offers))
        .routes(routes!(routes::trade::create_offer))
        .routes(routes!(routes::trade::get_offer))
        .routes(routes!(routes::trade::conversation_offers))
        .routes(routes!(routes::trade::reject_offer))
        .routes(routes!(routes::trade::cancel_offer))
        .routes(routes!(routes::trade::accept_offer))
        .routes(routes!(routes::orders::list))
        .routes(routes!(routes::orders::get))
        .routes(routes!(routes::orders::by_offer))
        .routes(routes!(routes::order_chat::list, routes::order_chat::send))
        .routes(routes!(routes::order_chat::read))
        .routes(routes!(routes::order_chat::verify))
        .routes(routes!(routes::order_chat::negotiation))
        .routes(routes!(routes::messages::list, routes::messages::open))
        .routes(routes!(routes::messages::get))
        .routes(routes!(routes::messages::messages, routes::messages::send))
        .routes(routes!(routes::messages::read))
        .routes(routes!(routes::messages::block, routes::messages::unblock))
        .routes(routes!(routes::presence::presence))
        .routes(routes!(routes::notifications::list))
        .routes(routes!(routes::notifications::read))
        .routes(routes!(routes::notifications::unread))
        .routes(routes!(routes::arbitration::arbitration_queue))
        .routes(routes!(routes::arbitration::arbitration_workload))
        .routes(routes!(routes::arbitration::get_case))
        .routes(routes!(routes::arbitration::order_case))
        .routes(routes!(routes::arbitration::add_case_statement))
        .routes(routes!(routes::arbitration::ruling_options))
        .routes(routes!(routes::arbitration::submit_ruling))
        .routes(routes!(routes::arbitration::appeal_case))
        .routes(routes!(routes::arbitration::halt_case))
        .routes(routes!(routes::arbitration::resume_case))
        .routes(routes!(routes::arbitration::execution_batch))
        .routes(routes!(routes::arbitration::public_ruling))
        .routes(routes!(routes::ruling_passkey::list_ruling_passkeys))
        .routes(routes!(routes::ruling_passkey::ruling_reverify_start))
        .routes(routes!(routes::ruling_passkey::ruling_reverify_verify))
        .routes(routes!(routes::ruling_passkey::ruling_register_start))
        .routes(routes!(routes::ruling_passkey::ruling_register_finish))
        .routes(routes!(routes::ruling_passkey::ruling_assert_start))
        .routes(routes!(routes::ruling_passkey::ruling_assert_finish))
        .routes(routes!(routes::ws::connect))
        .routes(routes!(routes::admin_auth::admin_enroll_code))
        .routes(routes!(routes::admin_auth::admin_enroll_verify))
        .routes(routes!(routes::admin_auth::admin_register_start))
        .routes(routes!(routes::admin_auth::admin_register_finish))
        .routes(routes!(routes::admin_auth::admin_passkeys))
        .routes(routes!(routes::admin_auth::admin_login_start))
        .routes(routes!(routes::admin_auth::admin_login_finish))
        .routes(routes!(routes::admin_auth::admin_current))
        .routes(routes!(routes::admin_auth::admin_reverify_start))
        .routes(routes!(routes::admin_auth::admin_reverify_finish))
        .routes(routes!(routes::admin_auth::admin_logout))
        .routes(routes!(routes::admin::list_staff, routes::admin::appoint))
        .routes(routes!(routes::admin::remove_staff))
        .routes(routes!(
            routes::admin::get_settings,
            routes::admin::put_settings
        ))
        .routes(routes!(routes::admin::list_suspensions))
        .routes(routes!(routes::admin::reinstate))
        .routes(routes!(routes::admin::revoke))
        .routes(routes!(routes::admin::audit_log))
        .split_for_parts()
}

/// 跨源只放行钱包页面与后台页面（plan 第 11 节「Web 安全基线」）：钱包子域名带 Cookie 调论坛主机上的 API，
/// 后台页面只调 `/api/v1/admin/`（后台来源的 CORS 只在后台接口上生效，钱包来源反之）。
/// 论坛页面与 API 同源，不需要 CORS；其他来源的请求不经过 CORS 层，拿不到任何 `Access-Control-*` 头
/// （`CorsLayer` 对不放行的来源仍会回 allow-methods 等头，所以先按来源分流）。
/// 会话 Cookie 的域与 SameSite 不变：钱包、后台与论坛同属一个可注册域名，跨子域名请求仍是同站。
#[derive(Clone)]
struct ApiCors {
    wallet_origins: Arc<[HeaderValue]>,
    wallet: CorsLayer,
    admin_origin: Option<HeaderValue>,
    admin: CorsLayer,
}

impl ApiCors {
    fn new(wallet_origins: &[String], admin_origin: &str) -> Self {
        // 来源在启动时已按 URL 校验过，必然是合法的请求头值。
        let wallet_origins: Arc<[HeaderValue]> = wallet_origins
            .iter()
            .filter_map(|o| HeaderValue::from_str(o).ok())
            .collect();
        let wallet = CorsLayer::new()
            .allow_origin(AllowOrigin::list(wallet_origins.iter().cloned()))
            .allow_credentials(true)
            .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::DELETE])
            .allow_headers([header::CONTENT_TYPE]);
        let admin_origin = HeaderValue::from_str(admin_origin).ok();
        let admin = CorsLayer::new()
            .allow_origin(AllowOrigin::list(admin_origin.iter().cloned()))
            .allow_credentials(true)
            .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
            .allow_headers([header::CONTENT_TYPE]);
        Self {
            wallet_origins,
            wallet,
            admin_origin,
            admin,
        }
    }
}

async fn api_cors(State(cors): State<ApiCors>, req: Request, next: Next) -> Response {
    let origin = req.headers().get(header::ORIGIN);
    let layer = if admin::is_admin_path(req.uri().path()) {
        origin
            .is_some_and(|o| cors.admin_origin.as_ref() == Some(o))
            .then_some(&cors.admin)
    } else {
        origin
            .is_some_and(|o| cors.wallet_origins.contains(o))
            .then_some(&cors.wallet)
    };
    match layer {
        None => next.run(req).await,
        Some(layer) => match layer.clone().layer(next).oneshot(req).await {
            Ok(res) => res,
            Err(never) => match never {},
        },
    }
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
    .layer(from_fn_with_state(
        ApiCors::new(&state.wallet.origins, &state.admin.origin),
        api_cors,
    ))
    .layer(TraceLayer::new_for_http())
    .layer(TimeoutLayer::with_status_code(
        StatusCode::REQUEST_TIMEOUT,
        Duration::from_secs(10),
    ))
    .with_state(state)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    /// 生成前端类型（openapi-typescript）要求每个 `$ref` 都能在 components 里找到、`operationId` 不重复
    /// （重名的接口会被合并成同一个类型）。
    #[test]
    fn openapi_refs_resolve_and_operation_ids_are_unique() {
        fn walk(v: &serde_json::Value, refs: &mut Vec<String>) {
            match v {
                serde_json::Value::Object(m) => {
                    if let Some(serde_json::Value::String(r)) = m.get("$ref") {
                        refs.push(r.clone());
                    }
                    m.values().for_each(|x| walk(x, refs));
                }
                serde_json::Value::Array(a) => a.iter().for_each(|x| walk(x, refs)),
                _ => {}
            }
        }
        let doc = serde_json::to_value(super::router().1).unwrap();
        let mut refs = Vec::new();
        walk(&doc, &mut refs);
        assert!(!refs.is_empty());
        for r in refs {
            let name = r.strip_prefix("#/components/schemas/").unwrap();
            assert!(
                doc["components"]["schemas"].get(name).is_some(),
                "unresolved {r}"
            );
        }
        let mut ids = std::collections::HashSet::new();
        for ops in doc["paths"].as_object().unwrap().values() {
            for op in ops.as_object().unwrap().values() {
                let id = op["operationId"].as_str().unwrap();
                assert!(ids.insert(id.to_owned()), "duplicate operationId {id}");
            }
        }
    }
}
