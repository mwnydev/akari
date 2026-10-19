//! forum-indexer 程序入口：读链配置（`CHAIN_CONFIG`）、`DATABASE_URL` 与可选的 `VALKEY_URL`，运行索引循环。

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
    let cache = match std::env::var("VALKEY_URL") {
        Ok(url) if !url.is_empty() => Some(forum_kv::ForumCache::new(forum_kv::pool(&url)?, "")),
        _ => {
            tracing::warn!("VALKEY_URL not set: user cards refresh only when their cache expires");
            None
        }
    };
    tracing::info!(version = env!("CARGO_PKG_VERSION"), chain = %cfg.name, "forum-indexer starting");
    forum_indexer::run(db, cfg, cache, shutdown).await?;
    Ok(())
}
