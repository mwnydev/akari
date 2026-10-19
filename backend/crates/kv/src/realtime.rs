//! 实时推送的 Valkey 部分（plan 第 7、8、11 节）：Pub/Sub 广播、WebSocket 连接登记（连接数上限与在线状态）。
//!
//! - **广播**：所有 forum-api 实例订阅同一个频道；每条消息 = 一行路由 JSON + 换行 + 事件 JSON。各实例按路由
//!   挑出本机的连接，把事件 JSON 原样转发（不再重新序列化）。发布方一律在数据库事务提交之后发布。
//! - **连接登记**：每个用户、每个 IP 一个有序集合，成员 = 连接 ID，分数 = 过期时间（毫秒）。心跳续期，
//!   实例崩溃留下的连接 [`CONN_TTL`] 后自动失效。用户集合里有未过期的连接即在线（plan「在线状态」）。

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use redis::Script;
use serde::{Deserialize, Serialize};

use crate::{KvError, Pool};

/// 连接登记的有效期：心跳每 30 秒续期，90 秒没有续期即视为断开。
pub const CONN_TTL: Duration = Duration::from_secs(90);
/// 最近活跃时间最多每分钟写一次库。
const SEEN_GATE: Duration = Duration::from_secs(60);

/// 订阅对象：会话或订单。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Topic {
    Conversation(i64),
    Order(i64),
}

/// 事件发给哪些连接。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "to", rename_all = "snake_case")]
pub enum Route {
    /// 这些用户的所有连接。
    Users { ids: Vec<i64> },
    /// 订单：买卖双方的所有连接，加上订阅了该订单的其他连接（仲裁员）。
    Order { id: i64, users: Vec<i64> },
    /// 只发给订阅了 `topic` 的连接，`except` 本人的连接除外（正在输入）。
    Subscribers { topic: Topic, except: i64 },
    /// 上下线：订阅了 `user_id` 参与的会话或订单的连接；`hidden`（隐身）时只发给订阅了进行中订单的交易对手。
    Presence { user_id: i64, hidden: bool },
}

/// 一条广播。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    pub route: Route,
    /// 事件 JSON（推给浏览器的原文）。
    pub event: String,
}

impl Envelope {
    pub fn new(route: Route, event: &impl Serialize) -> Result<Self, serde_json::Error> {
        Ok(Self {
            route,
            event: serde_json::to_string(event)?,
        })
    }

    fn encode(&self) -> Result<String, serde_json::Error> {
        Ok(format!(
            "{}\n{}",
            serde_json::to_string(&self.route)?,
            self.event
        ))
    }

    fn decode(raw: &str) -> Option<Self> {
        let (route, event) = raw.split_once('\n')?;
        Some(Self {
            route: serde_json::from_str(route).ok()?,
            event: event.to_owned(),
        })
    }
}

/// 新连接登记的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    /// 已登记；`first` 为真表示该用户此前不在线。
    Admitted {
        first: bool,
    },
    UserLimit,
    IpLimit,
}

const OPEN: &str = r"
local now = tonumber(ARGV[1])
local ttl = tonumber(ARGV[2])
redis.call('ZREMRANGEBYSCORE', KEYS[1], '-inf', now)
redis.call('ZREMRANGEBYSCORE', KEYS[2], '-inf', now)
local live = redis.call('ZCARD', KEYS[1])
if live >= tonumber(ARGV[4]) then return -1 end
if redis.call('ZCARD', KEYS[2]) >= tonumber(ARGV[5]) then return -2 end
for i = 1, 2 do
  redis.call('ZADD', KEYS[i], now + ttl, ARGV[3])
  redis.call('PEXPIRE', KEYS[i], ttl)
end
return live
";

const REFRESH: &str = r"
for i = 1, 2 do
  redis.call('ZADD', KEYS[i], tonumber(ARGV[1]) + tonumber(ARGV[2]), ARGV[3])
  redis.call('PEXPIRE', KEYS[i], ARGV[2])
end
return 1
";

const CLOSE: &str = r"
redis.call('ZREM', KEYS[1], ARGV[2])
redis.call('ZREM', KEYS[2], ARGV[2])
redis.call('ZREMRANGEBYSCORE', KEYS[1], '-inf', tonumber(ARGV[1]))
return redis.call('ZCARD', KEYS[1])
";

fn now_ms() -> i64 {
    i64::try_from(time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000)
        .unwrap_or(i64::MAX)
}

fn ttl_ms() -> u64 {
    u64::try_from(CONN_TTL.as_millis()).unwrap_or(u64::MAX)
}

/// 发布广播、登记连接、查询在线。`ns` 是键与频道前缀（[`crate::namespace_from_env`]；测试用随机前缀隔离）。
#[derive(Clone)]
pub struct Realtime {
    pool: Pool,
    ns: Arc<str>,
}

impl std::fmt::Debug for Realtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Realtime").field("ns", &self.ns).finish()
    }
}

impl Realtime {
    pub fn new(pool: Pool, ns: impl Into<Arc<str>>) -> Self {
        Self {
            pool,
            ns: ns.into(),
        }
    }

    /// 广播频道名。
    pub fn channel(&self) -> String {
        format!("{}ws:events", self.ns)
    }

    fn user_key(&self, user_id: i64) -> String {
        format!("{}ws:u:{user_id}", self.ns)
    }

    fn ip_key(&self, ip: &str) -> String {
        format!("{}ws:ip:{ip}", self.ns)
    }

    pub async fn publish(&self, e: &Envelope) -> Result<(), KvError> {
        let mut conn = self.pool.get().await?;
        let raw = e.encode().map_err(KvError::Encode)?;
        redis::cmd("PUBLISH")
            .arg(self.channel())
            .arg(raw)
            .query_async::<()>(&mut conn)
            .await?;
        Ok(())
    }

    /// 登记新连接；用户或 IP 的连接数已到上限时拒绝。
    pub async fn open(
        &self,
        user_id: i64,
        ip: &str,
        conn_id: &str,
        user_limit: u32,
        ip_limit: u32,
    ) -> Result<Admission, KvError> {
        let mut conn = self.pool.get().await?;
        let live: i64 = Script::new(OPEN)
            .key(self.user_key(user_id))
            .key(self.ip_key(ip))
            .arg(now_ms())
            .arg(ttl_ms())
            .arg(conn_id)
            .arg(user_limit)
            .arg(ip_limit)
            .invoke_async(&mut conn)
            .await?;
        Ok(match live {
            -1 => Admission::UserLimit,
            -2 => Admission::IpLimit,
            n => Admission::Admitted { first: n == 0 },
        })
    }

    /// 心跳续期。
    pub async fn refresh(&self, user_id: i64, ip: &str, conn_id: &str) -> Result<(), KvError> {
        let mut conn = self.pool.get().await?;
        Script::new(REFRESH)
            .key(self.user_key(user_id))
            .key(self.ip_key(ip))
            .arg(now_ms())
            .arg(ttl_ms())
            .arg(conn_id)
            .invoke_async::<()>(&mut conn)
            .await?;
        Ok(())
    }

    /// 注销连接；返回该用户是否已没有任何在线连接。
    pub async fn close(&self, user_id: i64, ip: &str, conn_id: &str) -> Result<bool, KvError> {
        let mut conn = self.pool.get().await?;
        let left: i64 = Script::new(CLOSE)
            .key(self.user_key(user_id))
            .key(self.ip_key(ip))
            .arg(now_ms())
            .arg(conn_id)
            .invoke_async(&mut conn)
            .await?;
        Ok(left == 0)
    }

    /// 批量查询是否在线（与 `ids` 一一对应）。
    pub async fn online(&self, ids: &[i64]) -> Result<Vec<bool>, KvError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut conn = self.pool.get().await?;
        let now = now_ms();
        let mut pipe = redis::pipe();
        for id in ids {
            pipe.cmd("ZCOUNT")
                .arg(self.user_key(*id))
                .arg(format!("({now}"))
                .arg("+inf");
        }
        let counts: Vec<i64> = pipe.query_async(&mut conn).await?;
        Ok(counts.into_iter().map(|n| n > 0).collect())
    }

    /// 最近活跃时间的写库闸门：一分钟内第一次调用返回真。
    pub async fn seen_gate(&self, user_id: i64) -> Result<bool, KvError> {
        let mut conn = self.pool.get().await?;
        let set: Option<String> = redis::cmd("SET")
            .arg(format!("{}ws:seen:{user_id}", self.ns))
            .arg(1)
            .arg("NX")
            .arg("PX")
            .arg(u64::try_from(SEEN_GATE.as_millis()).unwrap_or(u64::MAX))
            .query_async(&mut conn)
            .await?;
        Ok(set.is_some())
    }
}

/// 订阅广播频道，直到连接断开。订阅成功后先调用一次 `ready`，之后每条广播调用 `on`；无法解析的消息丢弃。
pub async fn listen(
    url: &str,
    channel: &str,
    ready: impl FnOnce(),
    mut on: impl FnMut(Envelope),
) -> Result<(), KvError> {
    let client = redis::Client::open(url)?;
    let mut pubsub = client.get_async_pubsub().await?;
    pubsub.subscribe(channel).await?;
    ready();
    let mut stream = pubsub.on_message();
    while let Some(msg) = stream.next().await {
        let Ok(raw) = msg.get_payload::<String>() else {
            continue;
        };
        match Envelope::decode(&raw) {
            Some(e) => on(e),
            None => tracing::warn!("undecodable realtime envelope dropped"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn envelope_roundtrip() {
        let e = Envelope::new(
            Route::Subscribers {
                topic: Topic::Order(7),
                except: 3,
            },
            &serde_json::json!({"type": "typing", "text": "a\nb"}),
        )
        .unwrap();
        let raw = e.encode().unwrap();
        assert!(raw.starts_with(r#"{"to":"subscribers","topic":{"order":7},"except":3}"#));
        assert_eq!(Envelope::decode(&raw), Some(e));
        assert_eq!(Envelope::decode("garbage"), None);
    }
}
