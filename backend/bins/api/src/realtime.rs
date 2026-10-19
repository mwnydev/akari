//! 本实例的 WebSocket 连接表与 Valkey 广播的分发（plan 第 7 节「订单即时通讯」、第 8 节 Valkey Pub/Sub）。
//!
//! 每个 forum-api 实例订阅同一个广播频道，按路由（[`Route`]）把事件转给本机的连接；发布方（本实例或别的实例、
//! worker）一律在数据库事务提交之后发布。广播订阅断线重连后给本机所有连接发 `{"type":"resync"}`：
//! 断线期间的广播已经丢了，客户端按 REST 补拉（见 [`crate::routes::ws`]）。

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use forum_core::chat::Event;
use forum_kv::Realtime;
use forum_kv::realtime::{Envelope, Route, Topic};
use tokio::sync::{mpsc, watch};

/// 每个连接的待发队列；满了说明客户端读得太慢，断开它（客户端重连后补拉）。
const QUEUE: usize = 256;
/// 每个连接最多同时订阅的会话 / 订单数。
pub const MAX_SUBSCRIPTIONS: usize = 20;
/// 广播订阅断线后的重连间隔。
const RECONNECT: Duration = Duration::from_secs(1);

/// 订阅的一个会话 / 订单里需要关注上下线的人：`(用户, 是否经由进行中订单)`。
pub type Watched = Vec<(i64, bool)>;

struct Local {
    user_id: i64,
    tx: mpsc::Sender<Arc<str>>,
    subs: HashMap<Topic, Watched>,
}

#[derive(Default)]
struct Registry {
    conns: HashMap<u64, Local>,
    by_user: HashMap<i64, HashSet<u64>>,
    by_topic: HashMap<Topic, HashSet<u64>>,
    /// 被关注的用户 → 关注他的连接 → 是否经由进行中订单（隐身用户只推给这些连接）。
    watchers: HashMap<i64, HashMap<u64, bool>>,
}

impl Registry {
    /// 投递到一个连接；队列满（客户端太慢）时移除该连接，它的任务随之收到 `None` 并关闭 socket。
    fn send(&mut self, conn: u64, event: &Arc<str>) {
        let full = self
            .conns
            .get(&conn)
            .is_some_and(|c| c.tx.try_send(event.clone()).is_err());
        if full {
            tracing::warn!(conn, "websocket queue full, dropping connection");
            self.remove(conn);
        }
    }

    fn unwatch(&mut self, conn: u64, subs: &HashMap<Topic, Watched>) {
        for (user, _) in subs.values().flatten() {
            if let Some(w) = self.watchers.get_mut(user) {
                w.remove(&conn);
                if w.is_empty() {
                    self.watchers.remove(user);
                }
            }
        }
    }

    fn watch(&mut self, conn: u64, subs: &HashMap<Topic, Watched>) {
        for (user, open) in subs.values().flatten() {
            let via_order = self
                .watchers
                .entry(*user)
                .or_default()
                .entry(conn)
                .or_default();
            *via_order |= *open;
        }
    }

    fn remove(&mut self, conn: u64) {
        let Some(local) = self.conns.remove(&conn) else {
            return;
        };
        if let Some(set) = self.by_user.get_mut(&local.user_id) {
            set.remove(&conn);
            if set.is_empty() {
                self.by_user.remove(&local.user_id);
            }
        }
        for topic in local.subs.keys() {
            if let Some(set) = self.by_topic.get_mut(topic) {
                set.remove(&conn);
                if set.is_empty() {
                    self.by_topic.remove(topic);
                }
            }
        }
        self.unwatch(conn, &local.subs);
    }

    fn targets(&self, route: &Route) -> Vec<u64> {
        let of_users = |ids: &[i64]| -> Vec<u64> {
            ids.iter()
                .filter_map(|u| self.by_user.get(u))
                .flatten()
                .copied()
                .collect()
        };
        let mut out = match route {
            Route::Users { ids } => of_users(ids),
            Route::Order { id, users } => {
                let mut t = of_users(users);
                t.extend(self.by_topic.get(&Topic::Order(*id)).into_iter().flatten());
                t
            }
            Route::Subscribers { topic, except } => self
                .by_topic
                .get(topic)
                .into_iter()
                .flatten()
                .copied()
                .filter(|c| self.conns.get(c).is_some_and(|l| l.user_id != *except))
                .collect(),
            Route::Presence { user_id, hidden } => self
                .watchers
                .get(user_id)
                .into_iter()
                .flatten()
                .filter(|(_, via_order)| !*hidden || **via_order)
                .map(|(c, _)| *c)
                .collect(),
        };
        out.sort_unstable();
        out.dedup();
        out
    }
}

pub struct Hub {
    rt: Realtime,
    url: String,
    next_id: AtomicU64,
    registry: Mutex<Registry>,
    ready: watch::Sender<bool>,
}

impl std::fmt::Debug for Hub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hub").field("rt", &self.rt).finish()
    }
}

impl Hub {
    /// `url` 是 Valkey 地址（广播订阅要单独一条连接，不走连接池）。调用 [`Hub::spawn`] 后才开始接收广播。
    pub fn new(rt: Realtime, url: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            rt,
            url: url.into(),
            next_id: AtomicU64::new(1),
            registry: Mutex::new(Registry::default()),
            ready: watch::Sender::new(false),
        })
    }

    pub fn realtime(&self) -> &Realtime {
        &self.rt
    }

    fn registry(&self) -> MutexGuard<'_, Registry> {
        // 持锁期间只做内存操作，不会 panic；中毒了也照常使用。
        self.registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// 启动广播订阅任务（断线自动重连）。
    pub fn spawn(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let hub = Arc::clone(self);
        tokio::spawn(async move {
            let channel = hub.rt.channel();
            let mut first = true;
            loop {
                let result = forum_kv::realtime::listen(
                    &hub.url,
                    &channel,
                    || {
                        if !first {
                            hub.resync_all();
                        }
                        hub.ready.send_replace(true);
                    },
                    |e| hub.dispatch(&e),
                )
                .await;
                first = false;
                hub.ready.send_replace(false);
                match result {
                    Ok(()) => tracing::warn!("realtime subscription closed, reconnecting"),
                    Err(e) => {
                        tracing::warn!(error = %e, "realtime subscription failed, reconnecting")
                    }
                }
                tokio::time::sleep(RECONNECT).await;
            }
        })
    }

    /// 等到广播订阅就绪（测试用：订阅前发布的广播收不到）。
    pub async fn ready(&self) {
        let mut rx = self.ready.subscribe();
        // 发送端在 Hub 里，Hub 活着就不会关闭。
        let _ = rx.wait_for(|r| *r).await;
    }

    fn resync_all(&self) {
        let event: Arc<str> = Arc::from(r#"{"type":"resync"}"#);
        let mut reg = self.registry();
        let ids: Vec<u64> = reg.conns.keys().copied().collect();
        for id in ids {
            reg.send(id, &event);
        }
    }

    /// 把一条广播转给本机的连接。
    pub fn dispatch(&self, e: &Envelope) {
        let event: Arc<str> = Arc::from(e.event.as_str());
        let mut reg = self.registry();
        for conn in reg.targets(&e.route) {
            reg.send(conn, &event);
        }
    }

    /// 登记本机连接，返回连接 ID 与它的待发队列。
    pub fn register(&self, user_id: i64) -> (u64, mpsc::Receiver<Arc<str>>) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel(QUEUE);
        let mut reg = self.registry();
        reg.conns.insert(
            id,
            Local {
                user_id,
                tx,
                subs: HashMap::new(),
            },
        );
        reg.by_user.entry(user_id).or_default().insert(id);
        (id, rx)
    }

    pub fn unregister(&self, conn: u64) {
        self.registry().remove(conn);
    }

    /// 订阅（调用方已检查权限）。超过 [`MAX_SUBSCRIPTIONS`] 返回 `false`；已订阅的更新关注列表。
    pub fn subscribe(&self, conn: u64, topic: Topic, watched: Watched) -> bool {
        let mut reg = self.registry();
        let Some(local) = reg.conns.get_mut(&conn) else {
            return false;
        };
        if !local.subs.contains_key(&topic) && local.subs.len() >= MAX_SUBSCRIPTIONS {
            return false;
        }
        let old = std::mem::take(&mut local.subs);
        let mut subs = old.clone();
        subs.insert(topic, watched);
        reg.unwatch(conn, &old);
        reg.watch(conn, &subs);
        if let Some(local) = reg.conns.get_mut(&conn) {
            local.subs = subs;
        }
        reg.by_topic.entry(topic).or_default().insert(conn);
        true
    }

    pub fn unsubscribe(&self, conn: u64, topic: Topic) {
        let mut reg = self.registry();
        let Some(local) = reg.conns.get_mut(&conn) else {
            return;
        };
        let old = std::mem::take(&mut local.subs);
        let mut subs = old.clone();
        subs.remove(&topic);
        reg.unwatch(conn, &old);
        reg.watch(conn, &subs);
        if let Some(local) = reg.conns.get_mut(&conn) {
            local.subs = subs;
        }
        if let Some(set) = reg.by_topic.get_mut(&topic) {
            set.remove(&conn);
            if set.is_empty() {
                reg.by_topic.remove(&topic);
            }
        }
    }

    pub fn is_subscribed(&self, conn: u64, topic: Topic) -> bool {
        self.registry()
            .conns
            .get(&conn)
            .is_some_and(|l| l.subs.contains_key(&topic))
    }

    /// 广播一个事件（在数据库事务提交之后调用）。失败只记日志：数据已在库里，客户端重连后会补拉。
    pub async fn publish(&self, route: Route, event: &Event) {
        let result = match Envelope::new(route, event) {
            Ok(e) => self.rt.publish(&e).await,
            Err(e) => Err(forum_kv::KvError::Encode(e)),
        };
        if let Err(e) = result {
            tracing::warn!(error = %e, "realtime publish failed");
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn hub() -> Arc<Hub> {
        let pool = forum_kv::pool("redis://127.0.0.1:1").unwrap();
        Hub::new(Realtime::new(pool, "t:"), "redis://127.0.0.1:1")
    }

    fn env(route: Route) -> Envelope {
        Envelope {
            route,
            event: "{}".into(),
        }
    }

    fn drain(rx: &mut mpsc::Receiver<Arc<str>>) -> usize {
        let mut n = 0;
        while rx.try_recv().is_ok() {
            n += 1;
        }
        n
    }

    #[test]
    fn routes_pick_the_right_connections() {
        let h = hub();
        let (a1, mut ra1) = h.register(1);
        let (_a2, mut ra2) = h.register(1);
        let (b, mut rb) = h.register(2);
        let (arb, mut rarb) = h.register(9);

        h.dispatch(&env(Route::Users { ids: vec![1] }));
        assert_eq!(
            (drain(&mut ra1), drain(&mut ra2), drain(&mut rb)),
            (1, 1, 0)
        );

        // 仲裁员订阅订单后收到订单事件；没订阅的人收不到
        assert!(h.subscribe(arb, Topic::Order(5), vec![(1, true), (2, true)]));
        h.dispatch(&env(Route::Order {
            id: 5,
            users: vec![1, 2],
        }));
        assert_eq!(
            (
                drain(&mut ra1),
                drain(&mut ra2),
                drain(&mut rb),
                drain(&mut rarb)
            ),
            (1, 1, 1, 1)
        );
        h.dispatch(&env(Route::Order {
            id: 6,
            users: vec![2],
        }));
        assert_eq!(
            (drain(&mut ra1), drain(&mut rb), drain(&mut rarb)),
            (0, 1, 0)
        );

        // 正在输入：只给订阅者，不给本人
        assert!(h.subscribe(a1, Topic::Conversation(3), vec![(2, false)]));
        assert!(h.subscribe(b, Topic::Conversation(3), vec![(1, false)]));
        h.dispatch(&env(Route::Subscribers {
            topic: Topic::Conversation(3),
            except: 2,
        }));
        assert_eq!(
            (drain(&mut ra1), drain(&mut ra2), drain(&mut rb)),
            (1, 0, 0)
        );

        // 上下线：隐身用户只推给经由进行中订单关注他的连接
        h.dispatch(&env(Route::Presence {
            user_id: 2,
            hidden: false,
        }));
        assert_eq!((drain(&mut ra1), drain(&mut rarb)), (1, 1));
        h.dispatch(&env(Route::Presence {
            user_id: 2,
            hidden: true,
        }));
        assert_eq!((drain(&mut ra1), drain(&mut rarb)), (0, 1));
        h.unsubscribe(arb, Topic::Order(5));
        h.dispatch(&env(Route::Presence {
            user_id: 2,
            hidden: true,
        }));
        assert_eq!(drain(&mut rarb), 0);

        h.unregister(a1);
        h.dispatch(&env(Route::Presence {
            user_id: 2,
            hidden: false,
        }));
        assert_eq!(drain(&mut ra1), 0);
        assert!(!h.is_subscribed(a1, Topic::Conversation(3)));
    }

    #[test]
    fn subscription_limit_and_slow_consumers() {
        let h = hub();
        let (c, mut rx) = h.register(1);
        for i in 0..MAX_SUBSCRIPTIONS {
            assert!(h.subscribe(c, Topic::Order(i as i64), Vec::new()));
        }
        assert!(!h.subscribe(c, Topic::Order(999), Vec::new()));
        assert!(
            h.subscribe(c, Topic::Order(0), Vec::new()),
            "re-subscribe is fine"
        );

        for _ in 0..=QUEUE {
            h.dispatch(&env(Route::Users { ids: vec![1] }));
        }
        assert_eq!(drain(&mut rx), QUEUE);
        assert!(
            rx.try_recv().is_err() && rx.is_closed(),
            "dropped when full"
        );
    }
}
