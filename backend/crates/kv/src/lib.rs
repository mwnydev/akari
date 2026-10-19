//! Valkey：连接池、跨进程滑动窗口限流、会话存储、论坛缓存、实时推送（Pub/Sub 与在线状态）。
//!
//! Valkey 只放可丢的数据（会话、验证码、限流计数、缓存），关键任务放 PostgreSQL（plan 第 2 节）。

pub mod ephemeral;
pub mod forum_cache;
pub mod ratelimit;
pub mod realtime;
pub mod session_store;

pub use deadpool_redis::{Connection, Pool};
pub use forum_cache::{BoardPage, ForumCache};
pub use ratelimit::SlidingWindow;
pub use realtime::Realtime;
pub use session_store::ValkeySessionStore;

#[derive(Debug, thiserror::Error)]
pub enum KvError {
    #[error("valkey pool: {0}")]
    Pool(#[from] deadpool_redis::PoolError),
    #[error("valkey: {0}")]
    Redis(#[from] redis::RedisError),
    #[error("valkey config: {0}")]
    Config(#[from] deadpool_redis::CreatePoolError),
    #[error("encode: {0}")]
    Encode(serde_json::Error),
    #[error("KV_PREFIX must be at most 32 characters of [A-Za-z0-9_-]")]
    InvalidPrefix,
}

/// 环境变量 `KV_PREFIX` 转成的键与频道前缀（[`ForumCache`]、[`Realtime`] 的 `ns`）。
///
/// Valkey 的键按 `VALKEY_URL` 里的数据库号隔离，Pub/Sub 频道却是整个实例共用的：共用一个 Valkey 的多个部署
/// （例如本机的多个检出）必须各设一个前缀，否则实时推送会串到别的部署。生产每台 Valkey 只有一个部署，留空。
pub fn namespace_from_env() -> Result<String, KvError> {
    namespace(std::env::var("KV_PREFIX").ok().as_deref())
}

/// `prefix` 为空时返回空串，否则为 `prefix:`。
pub fn namespace(prefix: Option<&str>) -> Result<String, KvError> {
    match prefix.unwrap_or_default() {
        "" => Ok(String::new()),
        p if p.len() <= 32
            && p.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') =>
        {
            Ok(format!("{p}:"))
        }
        _ => Err(KvError::InvalidPrefix),
    }
}

/// 建连接池；`url` 形如 `redis://127.0.0.1:6379` 或 `unix:///run/valkey/valkey.sock`。
pub fn pool(url: &str) -> Result<Pool, KvError> {
    Ok(deadpool_redis::Config::from_url(url).create_pool(Some(deadpool_redis::Runtime::Tokio1))?)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn namespace_from_prefix() {
        assert_eq!(namespace(None).unwrap(), "");
        assert_eq!(namespace(Some("")).unwrap(), "");
        assert_eq!(namespace(Some("chores-2_b")).unwrap(), "chores-2_b:");
        for bad in ["a:b", "a b", "ws*", "é", &"a".repeat(33)] {
            assert!(
                matches!(namespace(Some(bad)), Err(KvError::InvalidPrefix)),
                "{bad}"
            );
        }
    }
}
