//! forum-worker：后台任务：发件箱投递、apalis 队列（通知、超时处理、发薪）、图片转换。

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    forum_runtime::init_tracing();
    let shutdown = forum_runtime::shutdown_token();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "forum-worker started");
    shutdown.cancelled().await;
    tracing::info!("forum-worker stopped");
    Ok(())
}
