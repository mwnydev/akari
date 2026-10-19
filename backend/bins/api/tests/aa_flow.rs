//! `/api/v1/aa/rpc`：只接受钱包来源与已登录用户，只放行约定的方法，带用户 ID 与令牌转发给 forum-sponsor。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

mod common;

use axum::Json;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use common::*;
use forum_api::routes::aa::{SponsorProxy, USER_HEADER};
use forum_db::PgPool;
use serde_json::{Value, json};

const TOKEN: &str = "test-sponsor-token-0123456789abcdef";

/// 假的 forum-sponsor：核对令牌，把收到的用户 ID 与方法原样回显成 JSON-RPC 结果。
async fn fake_sponsor() -> String {
    async fn rpc(headers: HeaderMap, Json(body): Json<Value>) -> Result<Json<Value>, StatusCode> {
        let auth = headers.get("authorization").and_then(|v| v.to_str().ok());
        if auth != Some(&format!("Bearer {TOKEN}")) {
            return Err(StatusCode::UNAUTHORIZED);
        }
        let user = headers
            .get(USER_HEADER)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        Ok(Json(json!({
            "jsonrpc": "2.0",
            "id": body["id"],
            "result": {"user": user, "method": body["method"]},
        })))
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, axum::Router::new().route("/rpc", post(rpc)))
            .await
            .unwrap();
    });
    format!("http://{addr}")
}

fn rpc(method: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": 7, "method": method, "params": []})
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn forwards_authenticated_wallet_requests(db: PgPool) {
    let mut wallet = wallet_state();
    wallet.sponsor = Some(SponsorProxy::new(&fake_sponsor().await, TOKEN.to_owned()).unwrap());
    let Some(app) = build_with(db.clone(), wallet) else {
        return;
    };
    let (user_id, cookies) = user(&app, &db).await;
    let path = "/api/v1/aa/rpc";

    let r = wallet_call(&app, "POST", path, Some(rpc("eth_chainId")), &[]).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{:?}", r.body);

    let r = call(&app, "POST", path, Some(rpc("eth_chainId")), &cookies).await;
    assert_eq!(
        r.status,
        StatusCode::FORBIDDEN,
        "forum origin: {:?}",
        r.body
    );
    assert_eq!(r.body["code"], "ORIGIN_REJECTED");

    let r = wallet_call(
        &app,
        "POST",
        path,
        Some(rpc("eth_sendRawTransaction")),
        &cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{:?}", r.body);

    let r = wallet_call(
        &app,
        "POST",
        path,
        Some(rpc("pm_getPaymasterData")),
        &cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert_eq!(r.body["id"], 7);
    assert_eq!(r.body["result"]["user"], user_id.to_string());
    assert_eq!(r.body["result"]["method"], "pm_getPaymasterData");
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn signing_methods_are_rate_limited(db: PgPool) {
    let mut wallet = wallet_state();
    wallet.sponsor = Some(SponsorProxy::new(&fake_sponsor().await, TOKEN.to_owned()).unwrap());
    let Some(app) = build_with(db.clone(), wallet) else {
        return;
    };
    let (_, cookies) = user(&app, &db).await;
    let mut last = StatusCode::OK;
    for _ in 0..31 {
        let r = wallet_call(
            &app,
            "POST",
            "/api/v1/aa/rpc",
            Some(rpc("eth_sendUserOperation")),
            &cookies,
        )
        .await;
        last = r.status;
    }
    assert_eq!(last, StatusCode::TOO_MANY_REQUESTS);
    // 查询类方法不受签名限频影响
    let r = wallet_call(
        &app,
        "POST",
        "/api/v1/aa/rpc",
        Some(rpc("eth_chainId")),
        &cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn unconfigured_sponsor_is_unavailable(db: PgPool) {
    let Some(app) = build(db.clone()) else {
        return;
    };
    let (_, cookies) = user(&app, &db).await;
    let r = wallet_call(
        &app,
        "POST",
        "/api/v1/aa/rpc",
        Some(rpc("eth_chainId")),
        &cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::SERVICE_UNAVAILABLE, "{:?}", r.body);
    assert_eq!(r.body["code"], "SPONSOR_UNAVAILABLE");
}
