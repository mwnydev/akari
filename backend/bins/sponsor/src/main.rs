//! forum-sponsor：代付签名与私有代发：唯一持有 Paymaster 签名私钥和代发热钱包私钥的程序。

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    forum_runtime::init_tracing();
    let shutdown = forum_runtime::shutdown_token();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "forum-sponsor started");
    shutdown.cancelled().await;
    tracing::info!("forum-sponsor stopped");
    Ok(())
}
