//! forum-indexer：链上事件索引：链上状态进入论坛数据库的唯一入口（两级确认与区块回滚）。

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    forum_runtime::init_tracing();
    let shutdown = forum_runtime::shutdown_token();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "forum-indexer started");
    shutdown.cancelled().await;
    tracing::info!("forum-indexer stopped");
    Ok(())
}
