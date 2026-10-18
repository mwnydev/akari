//! forum-api：论坛 HTTP API 与 WebSocket。

use std::net::SocketAddr;
use std::time::Duration;

use axum::http::StatusCode;
use axum::{Json, Router, routing::get};
use serde::Serialize;
use tower_http::{timeout::TimeoutLayer, trace::TraceLayer};

#[derive(Serialize)]
struct Health {
    status: &'static str,
}

async fn healthz() -> Json<Health> {
    Json(Health { status: "ok" })
}

fn app() -> Router {
    Router::new()
        .route("/api/v1/healthz", get(healthz))
        .layer(TraceLayer::new_for_http())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(10),
        ))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    forum_runtime::init_tracing();
    let shutdown = forum_runtime::shutdown_token();
    let addr: SocketAddr = std::env::var("API_LISTEN")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_owned())
        .parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "forum-api listening");
    axum::serve(listener, app())
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await?;
    tracing::info!("forum-api stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn healthz_ok() {
        let res = super::app()
            .oneshot(Request::get("/api/v1/healthz").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["status"], "ok");
    }
}
