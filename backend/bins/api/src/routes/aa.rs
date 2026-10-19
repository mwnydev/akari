//! 钱包页面的 ERC-4337 / ERC-7677 JSON-RPC 入口（plan 第 5、11 节）：校验会话、钱包来源、限频后，
//! 带上用户 ID 转发给本机的 forum-sponsor。RPC 与 bundler 的服务商密钥只在服务器上，从不到浏览器。

use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::header;
use forum_core::ErrorCode;
use forum_kv::SlidingWindow;
use serde_json::Value;
use tower_sessions::Session;

use super::me::current_user_id;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::middleware::ClientIp;
use crate::state::AppState;
use crate::wallet::WalletOrigin;

/// 与 forum-sponsor 约定的用户 ID 请求头。
pub const USER_HEADER: &str = "x-forum-user-id";

/// 转发给 forum-sponsor 的方法；其他一律拒绝。
pub const METHODS: &[&str] = &[
    "pm_getPaymasterStubData",
    "pm_getPaymasterData",
    "eth_estimateUserOperationGas",
    "eth_sendUserOperation",
    "eth_getUserOperationReceipt",
    "eth_getUserOperationByHash",
    "eth_supportedEntryPoints",
    "eth_chainId",
    "pimlico_getUserOperationGasPrice",
];

/// 等回执时钱包每秒左右轮询一次，所以总量给得宽；签名与提交单独收紧（每次都要模拟、耗 RPC）。
const PER_USER: SlidingWindow = SlidingWindow::new("aa-rpc-user", 300, Duration::from_secs(300));
const PER_IP: SlidingWindow = SlidingWindow::new("aa-rpc-ip", 600, Duration::from_secs(300));
const SIGN_PER_USER: SlidingWindow =
    SlidingWindow::new("aa-sign-user", 30, Duration::from_secs(600));

/// 本机 forum-sponsor 的地址与共享令牌（`SPONSOR_URL`、`SPONSOR_API_TOKEN`）。
#[derive(Debug, Clone)]
pub struct SponsorProxy {
    client: reqwest::Client,
    url: String,
    token: String,
}

impl SponsorProxy {
    /// `url` 形如 `http://127.0.0.1:8090`；只走本机，不经 `HTTP_PROXY`。
    pub fn new(url: &str, token: String) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            client,
            url: format!("{}/rpc", url.trim_end_matches('/')),
            token,
        })
    }

    async fn forward(&self, user_id: i64, body: &Value) -> ApiResult<Value> {
        let unavailable = |e: reqwest::Error| {
            tracing::error!(error = %e, "forum-sponsor unreachable");
            ApiError(ErrorCode::SponsorUnavailable)
        };
        let res = self
            .client
            .post(&self.url)
            .bearer_auth(&self.token)
            .header(USER_HEADER, user_id)
            .header(header::CONTENT_TYPE, "application/json")
            .json(body)
            .send()
            .await
            .map_err(unavailable)?;
        if !res.status().is_success() {
            tracing::error!(status = %res.status(), "forum-sponsor refused the request");
            return Err(ApiError(ErrorCode::SponsorUnavailable));
        }
        res.json().await.map_err(unavailable)
    }
}

/// 按任意标识（IP、用户 ID）限频。
pub(crate) async fn limit(state: &AppState, rule: &SlidingWindow, id: &str) -> ApiResult<()> {
    if rule
        .hit(&state.kv, &format!("{}{id}", state.cache.ns()))
        .await?
    {
        Ok(())
    } else {
        Err(ApiError(ErrorCode::RateLimited))
    }
}

/// 单个 JSON-RPC 请求（不支持批量）；返回 forum-sponsor 的 JSON-RPC 响应。
#[utoipa::path(
    post, path = "/api/v1/aa/rpc", tag = "wallet",
    request_body = Object,
    responses(
        (status = 200, body = Object, description = "JSON-RPC 2.0 响应（成功或 error）"),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 429, body = ErrorBody), (status = 503, body = ErrorBody)
    )
)]
pub async fn rpc(
    State(state): State<AppState>,
    _origin: WalletOrigin,
    ClientIp(ip): ClientIp,
    session: Session,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let user_id = current_user_id(&session).await?;
    let method = body
        .get("method")
        .and_then(Value::as_str)
        .filter(|m| METHODS.contains(m))
        .ok_or(ApiError(ErrorCode::InvalidParameter))?;
    limit(&state, &PER_IP, &ip.to_string()).await?;
    limit(&state, &PER_USER, &user_id.to_string()).await?;
    if matches!(method, "pm_getPaymasterData" | "eth_sendUserOperation") {
        limit(&state, &SIGN_PER_USER, &user_id.to_string()).await?;
    }
    let proxy = state
        .wallet
        .sponsor
        .as_ref()
        .ok_or(ApiError(ErrorCode::SponsorUnavailable))?;
    Ok(Json(proxy.forward(user_id, &body).await?))
}
