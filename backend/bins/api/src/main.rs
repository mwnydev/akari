//! forum-api 程序入口：读配置、建连接、启动 HTTP 服务。

use std::net::SocketAddr;
use std::sync::Arc;

use axum_extra::extract::cookie::Key;
use forum_api::config::Config;
use forum_api::{AppState, app};
use forum_auth::{AnyCaptcha, CodeStore, NoopCaptcha, Turnstile};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    forum_runtime::init_tracing();
    forum_runtime::install_crypto_provider();
    let shutdown = forum_runtime::shutdown_token();
    let cfg = Config::from_env()?;

    let db = forum_db::connect(&cfg.database_url, 20).await?;
    let kv = forum_kv::pool(&cfg.valkey_url)?;
    let captcha = match &cfg.turnstile_secret {
        Some(secret) => {
            AnyCaptcha::Turnstile(Turnstile::new(reqwest::Client::new(), secret.clone()))
        }
        None => {
            tracing::warn!("TURNSTILE_SECRET not set: captcha disabled (development only)");
            AnyCaptcha::Noop(NoopCaptcha)
        }
    };
    let state = AppState {
        db,
        kv: kv.clone(),
        codes: CodeStore::new(kv, &cfg.auth_secret)?,
        captcha,
        // 设备 Cookie 与会话 Cookie 用同一把 64 字节密钥派生的不同 Key 类型。
        device_key: Key::try_from(cfg.cookie_secret.as_slice())?,
        allowed_origins: Arc::from(cfg.allowed_origins.clone()),
        cookie_secure: cfg.cookie_secure,
    };

    let addr: SocketAddr = cfg.listen.parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "forum-api listening");
    axum::serve(
        listener,
        app(state, &cfg.cookie_secret).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move { shutdown.cancelled().await })
    .await?;
    tracing::info!("forum-api stopped");
    Ok(())
}
