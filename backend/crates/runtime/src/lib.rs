//! 四个程序共用的启动组件：结构化日志、退出信号。

use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

/// 初始化 tracing：`RUST_LOG` 控制级别（默认 info），`LOG_FORMAT=json` 输出 JSON。
pub fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    if std::env::var("LOG_FORMAT").is_ok_and(|v| v == "json") {
        builder.json().init();
    } else {
        builder.init();
    }
}

/// 返回一个在收到 Ctrl-C 或 SIGTERM 时取消的令牌；所有后台任务监听它优雅退出。
pub fn shutdown_token() -> CancellationToken {
    let token = CancellationToken::new();
    let t = token.clone();
    tokio::spawn(async move {
        wait_for_signal().await;
        tracing::info!("shutdown signal received");
        t.cancel();
    });
    token
}

async fn wait_for_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::error!(error = %e, "failed to listen for ctrl-c");
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(e) => {
                tracing::error!(error = %e, "failed to listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = term => {}
    }
}
