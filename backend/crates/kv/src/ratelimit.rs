//! 跨进程滑动窗口限流（plan 第 2 节：Rust 生态没有成熟的跨进程限流库，只自写这几十行）。
//!
//! 每个键是一个有序集合，成员 = 本次请求的唯一序号，分数 = 毫秒时间戳。
//! 一段 Lua 原子完成：清掉窗口外的记录 → 计数 → 未超限才记入本次。被拒绝的请求不占名额。

use std::time::Duration;

use redis::Script;

use crate::{KvError, Pool};

const SCRIPT: &str = r"
local key = KEYS[1]
local now = tonumber(ARGV[1])
local window = tonumber(ARGV[2])
local limit = tonumber(ARGV[3])
redis.call('ZREMRANGEBYSCORE', key, '-inf', now - window)
if redis.call('ZCARD', key) >= limit then
  return 0
end
local seq = redis.call('INCR', key .. ':seq')
redis.call('PEXPIRE', key .. ':seq', window)
redis.call('ZADD', key, now, seq)
redis.call('PEXPIRE', key, window)
return 1
";

#[derive(Debug, Clone)]
pub struct SlidingWindow {
    prefix: &'static str,
    limit: u32,
    window: Duration,
}

impl SlidingWindow {
    pub const fn new(prefix: &'static str, limit: u32, window: Duration) -> Self {
        Self {
            prefix,
            limit,
            window,
        }
    }

    /// 记一次请求；返回 `true` 表示放行，`false` 表示已超限。
    pub async fn hit(&self, pool: &Pool, id: &str) -> Result<bool, KvError> {
        let mut conn = pool.get().await?;
        let now_ms = time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000;
        let window_ms = i64::try_from(self.window.as_millis()).unwrap_or(i64::MAX);
        let allowed: i64 = Script::new(SCRIPT)
            .key(format!("rl:{}:{id}", self.prefix))
            .arg(i64::try_from(now_ms).unwrap_or(i64::MAX))
            .arg(window_ms)
            .arg(self.limit)
            .invoke_async(&mut conn)
            .await?;
        Ok(allowed == 1)
    }
}
