//! forum-indexer 程序入口：读链配置（`CHAIN_CONFIG`）与 `DATABASE_URL`，运行索引循环。

use anyhow::Context;
use forum_chain::config::ChainConfig;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    forum_runtime::init_tracing();
    forum_runtime::install_crypto_provider();
    let shutdown = forum_runtime::shutdown_token();

    let path = std::env::var("CHAIN_CONFIG").context("missing env CHAIN_CONFIG")?;
    let cfg = ChainConfig::load(path.as_ref())?;
    let db = forum_db::connect(
        &std::env::var("DATABASE_URL").context("missing env DATABASE_URL")?,
        4,
    )
    .await?;
    tracing::info!(version = env!("CARGO_PKG_VERSION"), chain = %cfg.name, "forum-indexer starting");
    forum_indexer::run(db, cfg, shutdown).await?;
    Ok(())
}
