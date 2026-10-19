//! Valkey：连接池、跨进程滑动窗口限流、会话存储、论坛缓存。
//!
//! Valkey 只放可丢的数据（会话、验证码、限流计数、缓存），关键任务放 PostgreSQL（plan 第 2 节）。

pub mod forum_cache;
pub mod ratelimit;
pub mod session_store;

pub use deadpool_redis::{Connection, Pool};
pub use forum_cache::{BoardPage, ForumCache};
pub use ratelimit::SlidingWindow;
pub use session_store::ValkeySessionStore;

#[derive(Debug, thiserror::Error)]
pub enum KvError {
    #[error("valkey pool: {0}")]
    Pool(#[from] deadpool_redis::PoolError),
    #[error("valkey: {0}")]
    Redis(#[from] redis::RedisError),
    #[error("valkey config: {0}")]
    Config(#[from] deadpool_redis::CreatePoolError),
}

/// 建连接池；`url` 形如 `redis://127.0.0.1:6379` 或 `unix:///run/valkey/valkey.sock`。
pub fn pool(url: &str) -> Result<Pool, KvError> {
    Ok(deadpool_redis::Config::from_url(url).create_pool(Some(deadpool_redis::Runtime::Tokio1))?)
}
