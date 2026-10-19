//! 短期状态：一次性的（如 WebAuthn challenge）写入带过期时间、取出即删除，防止重放；
//! 几秒的读缓存（如链上读代理）用 [`get`] 读取，到期自动消失。

use std::time::Duration;

use crate::{KvError, Pool};

/// 写入并设置过期时间；同一个键再次写入会覆盖旧值（旧 challenge 随之作废）。
pub async fn put(pool: &Pool, key: &str, value: &str, ttl: Duration) -> Result<(), KvError> {
    let mut conn = pool.get().await?;
    redis::cmd("SET")
        .arg(key)
        .arg(value)
        .arg("PX")
        .arg(u64::try_from(ttl.as_millis()).unwrap_or(u64::MAX))
        .query_async::<()>(&mut conn)
        .await?;
    Ok(())
}

/// 原子地取出并删除；不存在或已过期返回 `None`。
pub async fn take(pool: &Pool, key: &str) -> Result<Option<String>, KvError> {
    let mut conn = pool.get().await?;
    Ok(redis::cmd("GETDEL").arg(key).query_async(&mut conn).await?)
}

/// 读取但不删除；不存在或已过期返回 `None`。
pub async fn get(pool: &Pool, key: &str) -> Result<Option<String>, KvError> {
    let mut conn = pool.get().await?;
    Ok(redis::cmd("GET").arg(key).query_async(&mut conn).await?)
}
