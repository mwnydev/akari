//! forum-sponsor 程序入口：读配置与私钥、核对链上 Paymaster 与拍卖签名者、启动代发器并监听本机端口。

use forum_sponsor::config::Config;
use forum_sponsor::{App, keys};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    forum_runtime::init_tracing();
    forum_runtime::install_crypto_provider();
    let shutdown = forum_runtime::shutdown_token();
    let cfg = Config::from_env()?;
    let paymaster_signer = keys::PAYMASTER_SIGNER.load(cfg.chain.chain_id)?;
    let hot_wallet = keys::HOT_WALLET.load(cfg.chain.chain_id)?;
    let auction_signer = keys::AUCTION_SIGNER.load_optional(cfg.chain.chain_id)?;
    if auction_signer.is_none() {
        tracing::warn!("credential auction-signer not set: auction settlement proofs disabled");
    }
    let db = forum_db::connect(&cfg.database_url, 4).await?;
    let listen = cfg.listen;
    let app = App::new(cfg, db, paymaster_signer, hot_wallet, auction_signer)?;
    app.verify_onchain().await?;
    let tasks = app.spawn_background(&shutdown);

    let listener = tokio::net::TcpListener::bind(listen).await?;
    tracing::info!(version = env!("CARGO_PKG_VERSION"), %listen, "forum-sponsor listening");
    let stop = shutdown.clone();
    axum::serve(listener, app.router())
        .with_graceful_shutdown(async move { stop.cancelled().await })
        .await?;
    for t in tasks {
        let _ = t.await;
    }
    tracing::info!("forum-sponsor stopped");
    Ok(())
}
