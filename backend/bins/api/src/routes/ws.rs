//! WebSocket 实时推送 `GET /api/v1/ws`（plan 第 7、11 节）。
//!
//! **建立连接**：必须已登录；`Origin` 必须是论坛或钱包页面；每个用户最多 5 条、每个 IP 最多 20 条连接
//! （两个 forum-api 实例合计，超出返回 429）。连上后服务端先发 `{"type":"ready","user_id":…,"heartbeat_secs":30}`。
//!
//! **心跳**：服务端每 30 秒发一个 Ping（浏览器自动回 Pong），客户端也可以发 `{"type":"ping"}`（回 `{"type":"pong"}`）。
//! 90 秒内没收到客户端任何帧即断开。有连接即在线；心跳同时续期在线状态。
//!
//! **服务端推送**（`type`）：`message`、`read`（私信）；`order_message`、`order_read`、`order`（订单）；
//! `notification`；`presence`、`typing`（只在订阅的会话 / 订单里）；`comment`、`comment_removed`、`likes`
//! （只在订阅的主题页里）；`resync`（推送通道断过，见下）。
//! 结构见 `forum_core::chat::Event`。
//!
//! **客户端消息**：
//! - `{"type":"subscribe","conversation_id":1}` / `{"type":"subscribe","order_id":2}`：打开会话或订单页时订阅
//!   （检查权限；仲裁员要订阅才收得到订单推送），回 `{"type":"subscribed",…,"presence":[对方在线状态]}`；
//!   之后收到对方上下线与正在输入。`unsubscribe` 同理。每条连接最多订阅 20 个（会话、订单、主题页合计）。
//! - `{"type":"subscribe","topic_id":3}`：打开主题页时订阅（检查主题可见性），之后收到楼层下的新评论、
//!   删除评论与点赞数变化（`presence` 为空）。主题被隐藏后不再推送。
//! - `{"type":"typing","conversation_id":1}`（或 `order_id`）：只能在已订阅的会话 / 订单里发，每 2 秒最多一次。
//!
//! **断线重连与补拉**：推送只负责「尽快」，不保证送达。客户端记住每个打开的会话 / 订单的最后一条消息 ID 和最新通知 ID，
//! 重连（或收到 `resync`）后先重新订阅，再用 REST 补拉：
//! `GET /api/v1/conversations/{id}/messages?after=<最后 ID>`、`GET /api/v1/orders/{id}/messages?after=<最后 ID>`、
//! `GET /api/v1/notifications?after=<最新 ID>`（都按时间正序，`has_more` / `next_cursor` 为真就继续拉），
//! 订单状态用 `GET /api/v1/orders/{id}`，主题页重新取当前页。补拉与推送可能重复，按消息 / 评论 ID 去重。

use std::time::{Duration, Instant};

use axum::extract::State;
use axum::extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::http::HeaderMap;
use axum::response::Response;
use forum_core::ErrorCode;
use forum_core::chat::{Event, Presence};
use forum_kv::realtime::{Admission, CONN_TTL, Route, Topic};
use futures_util::{SinkExt, StreamExt};
use rand::RngExt;
use serde::{Deserialize, Serialize};
use tower_sessions::Session;

use super::me::current_user_id;
use super::{order_chat, presence};
use crate::error::{ApiError, ApiResult};
use crate::middleware::{ClientIp, origin_allowed};
use crate::realtime::Watched;
use crate::state::AppState;

/// 每个用户、每个 IP 的连接数上限（plan 第 11 节「WebSocket 被刷」）。
pub const MAX_CONNS_PER_USER: u32 = 5;
pub const MAX_CONNS_PER_IP: u32 = 20;
pub const HEARTBEAT: Duration = Duration::from_secs(30);
/// 客户端帧最大字节数（只有订阅、输入状态这类小消息）。
const MAX_FRAME: usize = 4096;
/// 每 10 秒最多接收的客户端帧数，超过即断开。
const FRAME_BUDGET: u32 = 50;
const FRAME_WINDOW: Duration = Duration::from_secs(10);
const TYPING_INTERVAL: Duration = Duration::from_secs(2);
/// 自定义关闭码：客户端读得太慢 / 发得太多。
const CLOSE_SLOW: u16 = 4008;
const CLOSE_POLICY: u16 = 1008;

#[derive(Debug, Deserialize)]
struct Target {
    conversation_id: Option<i64>,
    order_id: Option<i64>,
    topic_id: Option<i64>,
}

impl Target {
    fn topic(&self) -> Option<Topic> {
        match (self.conversation_id, self.order_id, self.topic_id) {
            (Some(c), None, None) => Some(Topic::Conversation(c)),
            (None, Some(o), None) => Some(Topic::Order(o)),
            (None, None, Some(t)) => Some(Topic::Thread(t)),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMsg {
    Ping,
    Subscribe(Target),
    Unsubscribe(Target),
    Typing(Target),
}

/// 只回给本连接的消息（不经广播）。
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Reply {
    Ready {
        user_id: i64,
        heartbeat_secs: u64,
    },
    Pong,
    Subscribed {
        #[serde(flatten)]
        target: TargetIds,
        presence: Vec<Presence>,
    },
    Unsubscribed {
        #[serde(flatten)]
        target: TargetIds,
    },
    Error {
        code: ErrorCode,
    },
}

/// 回复里标明订阅对象（对应的那个字段有值，其余为空）。
#[derive(Debug, Default, Serialize)]
struct TargetIds {
    conversation_id: Option<i64>,
    order_id: Option<i64>,
    topic_id: Option<i64>,
}

fn ids_of(topic: Topic) -> TargetIds {
    match topic {
        Topic::Conversation(c) => TargetIds {
            conversation_id: Some(c),
            ..TargetIds::default()
        },
        Topic::Order(o) => TargetIds {
            order_id: Some(o),
            ..TargetIds::default()
        },
        Topic::Thread(t) => TargetIds {
            topic_id: Some(t),
            ..TargetIds::default()
        },
    }
}

/// 建立 WebSocket 连接（协议见模块文档）。
#[utoipa::path(
    get, path = "/api/v1/ws", tag = "realtime", operation_id = "websocket",
    responses(
        (status = 101, description = "已升级为 WebSocket；协议见 forum-api `routes::ws` 模块文档"),
        (status = 401, body = crate::error::ErrorBody), (status = 403, body = crate::error::ErrorBody),
        (status = 429, body = crate::error::ErrorBody)
    )
)]
pub async fn connect(
    State(state): State<AppState>,
    session: Session,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> ApiResult<Response> {
    if !origin_allowed(&state, &headers) {
        return Err(ApiError(ErrorCode::OriginRejected));
    }
    let user_id = current_user_id(&session).await?;
    let conn_key = hex::encode(rand::rng().random::<[u8; 16]>());
    let ip = ip.to_string();
    let rt = state.hub.realtime().clone();
    let first = match rt
        .open(
            user_id,
            &ip,
            &conn_key,
            MAX_CONNS_PER_USER,
            MAX_CONNS_PER_IP,
        )
        .await?
    {
        Admission::Admitted { first } => first,
        Admission::UserLimit | Admission::IpLimit => return Err(ApiError(ErrorCode::RateLimited)),
    };
    let failed = (rt.clone(), user_id, ip.clone(), conn_key.clone());
    Ok(ws
        .max_message_size(MAX_FRAME)
        .max_frame_size(MAX_FRAME)
        .on_failed_upgrade(move |e| {
            tracing::debug!(error = %e, "websocket upgrade failed");
            let (rt, user_id, ip, key) = failed;
            tokio::spawn(async move {
                if let Err(e) = rt.close(user_id, &ip, &key).await {
                    tracing::warn!(error = %e, "websocket registry cleanup failed");
                }
            });
        })
        .on_upgrade(move |socket| async move {
            let conn = Connection {
                state,
                user_id,
                ip,
                key: conn_key,
                id: 0,
            };
            conn.run(socket, first).await;
        }))
}

struct Connection {
    state: AppState,
    user_id: i64,
    ip: String,
    /// Valkey 连接登记里的 ID（跨实例唯一）。
    key: String,
    /// 本实例连接表里的 ID。
    id: u64,
}

impl Connection {
    async fn run(mut self, socket: WebSocket, first: bool) {
        let (id, mut events) = self.state.hub.register(self.user_id);
        self.id = id;
        let (mut sink, mut stream) = socket.split();
        if first {
            self.announce(true).await;
        }
        let ready = Reply::Ready {
            user_id: self.user_id,
            heartbeat_secs: HEARTBEAT.as_secs(),
        };
        let mut heartbeat =
            tokio::time::interval_at(tokio::time::Instant::now() + HEARTBEAT, HEARTBEAT);
        let mut last_frame = Instant::now();
        let mut window = (Instant::now(), 0_u32);
        let mut last_typing: Option<Instant> = None;
        let mut close = None;
        if sink.send(text(&ready)).await.is_ok() {
            close = loop {
                tokio::select! {
                    _ = heartbeat.tick() => {
                        if last_frame.elapsed() > CONN_TTL {
                            break Some((CLOSE_POLICY, "heartbeat timeout"));
                        }
                        if sink.send(Message::Ping(Default::default())).await.is_err() {
                            break None;
                        }
                        self.beat().await;
                    }
                    event = events.recv() => match event {
                        Some(json) => {
                            if sink.send(Message::Text(Utf8Bytes::from(&*json))).await.is_err() {
                                break None;
                            }
                        }
                        // 连接表因待发队列满把本连接移除了。
                        None => break Some((CLOSE_SLOW, "too slow")),
                    },
                    frame = stream.next() => {
                        let Some(Ok(frame)) = frame else { break None };
                        last_frame = Instant::now();
                        if window.0.elapsed() > FRAME_WINDOW {
                            window = (Instant::now(), 0);
                        }
                        window.1 += 1;
                        if window.1 > FRAME_BUDGET {
                            break Some((CLOSE_POLICY, "too many frames"));
                        }
                        let raw = match frame {
                            Message::Text(t) => t,
                            Message::Close(_) => break None,
                            // Ping 由 axum 自动回 Pong；Pong 只用来证明连接活着。
                            Message::Ping(_) | Message::Pong(_) => continue,
                            Message::Binary(_) => break Some((CLOSE_POLICY, "binary frames not accepted")),
                        };
                        let reply = match serde_json::from_str::<ClientMsg>(raw.as_str()) {
                            Ok(msg) => self.handle(msg, &mut last_typing).await,
                            Err(_) => Some(Reply::Error { code: ErrorCode::InvalidParameter }),
                        };
                        if let Some(r) = reply
                            && sink.send(text(&r)).await.is_err()
                        {
                            break None;
                        }
                    }
                }
            };
        }
        if let Some((code, reason)) = close {
            let _ = sink
                .send(Message::Close(Some(CloseFrame {
                    code,
                    reason: reason.into(),
                })))
                .await;
        }
        self.finish().await;
    }

    /// 心跳：续期连接登记，并按闸门（每分钟一次）写最近活跃时间。
    async fn beat(&self) {
        let rt = self.state.hub.realtime();
        if let Err(e) = rt.refresh(self.user_id, &self.ip, &self.key).await {
            tracing::warn!(error = %e, "websocket heartbeat refresh failed");
        }
        match rt.seen_gate(self.user_id).await {
            Ok(true) => {
                if let Err(e) = forum_db::users::touch_last_seen(&self.state.db, self.user_id).await
                {
                    tracing::warn!(error = %e, "last_seen_at update failed");
                }
            }
            Ok(false) => {}
            Err(e) => tracing::warn!(error = %e, "last_seen gate failed"),
        }
    }

    async fn finish(self) {
        self.state.hub.unregister(self.id);
        if let Err(e) = forum_db::users::touch_last_seen(&self.state.db, self.user_id).await {
            tracing::warn!(error = %e, "last_seen_at update failed");
        }
        match self
            .state
            .hub
            .realtime()
            .close(self.user_id, &self.ip, &self.key)
            .await
        {
            Ok(true) => self.announce(false).await,
            Ok(false) => {}
            Err(e) => tracing::warn!(error = %e, "websocket registry cleanup failed"),
        }
    }

    /// 上下线广播（推给订阅了相关会话 / 订单的连接；隐身时只推给进行中订单的交易对手）。
    async fn announce(&self, online: bool) {
        let row = match forum_db::users::visibility(&self.state.db, &[self.user_id]).await {
            Ok(mut rows) => rows.pop(),
            Err(e) => {
                tracing::warn!(error = %e, "presence lookup failed");
                None
            }
        };
        let Some(row) = row else { return };
        self.state
            .hub
            .publish(
                Route::Presence {
                    user_id: self.user_id,
                    hidden: !row.show_online,
                },
                &Event::Presence(Presence::of(self.user_id, online, row.last_seen_at, true)),
            )
            .await;
    }

    async fn handle(&self, msg: ClientMsg, last_typing: &mut Option<Instant>) -> Option<Reply> {
        let result = match msg {
            ClientMsg::Ping => return Some(Reply::Pong),
            ClientMsg::Subscribe(t) => self.subscribe(&t).await,
            ClientMsg::Unsubscribe(t) => match t.topic() {
                Some(topic) => {
                    self.state.hub.unsubscribe(self.id, topic);
                    Ok(Some(Reply::Unsubscribed {
                        target: ids_of(topic),
                    }))
                }
                None => Err(ApiError(ErrorCode::InvalidParameter)),
            },
            ClientMsg::Typing(t) => self.typing(&t, last_typing).await.map(|()| None),
        };
        result.unwrap_or_else(|e| Some(Reply::Error { code: e.0 }))
    }

    async fn subscribe(&self, t: &Target) -> ApiResult<Option<Reply>> {
        let topic = t.topic().ok_or(ApiError(ErrorCode::InvalidParameter))?;
        let me = self.user_id;
        let watched: Watched = match topic {
            Topic::Conversation(id) => {
                let c = forum_db::messages::conversation(&self.state.db, id)
                    .await?
                    .filter(|c| c.is_member(me))
                    .ok_or(ApiError(ErrorCode::NotFound))?;
                vec![(c.peer(me), false)]
            }
            Topic::Order(id) => {
                let p = order_chat::accessible(&self.state, id, me).await?;
                p.users()
                    .into_iter()
                    .filter(|u| *u != me)
                    .map(|u| (u, p.open()))
                    .collect()
            }
            Topic::Thread(id) => {
                let viewer = crate::access::Viewer {
                    user: forum_db::users::actor(&self.state.db, me).await?,
                };
                crate::access::topic_access(&self.state, &viewer, id).await?;
                Vec::new()
            }
        };
        if !self.state.hub.subscribe(self.id, topic, watched.clone()) {
            return Err(ApiError(ErrorCode::RateLimited));
        }
        let ids: Vec<i64> = watched.iter().map(|(u, _)| *u).collect();
        Ok(Some(Reply::Subscribed {
            target: ids_of(topic),
            presence: presence::lookup(&self.state, Some(me), &ids).await?,
        }))
    }

    async fn typing(&self, t: &Target, last: &mut Option<Instant>) -> ApiResult<()> {
        let topic = t.topic().ok_or(ApiError(ErrorCode::InvalidParameter))?;
        let (conversation_id, order_id) = match topic {
            Topic::Conversation(c) => (Some(c), None),
            Topic::Order(o) => (None, Some(o)),
            Topic::Thread(_) => return Err(ApiError(ErrorCode::InvalidParameter)),
        };
        if !self.state.hub.is_subscribed(self.id, topic) {
            return Err(ApiError(ErrorCode::Forbidden));
        }
        if last.is_some_and(|at| at.elapsed() < TYPING_INTERVAL) {
            return Ok(());
        }
        *last = Some(Instant::now());
        self.state
            .hub
            .publish(
                Route::Subscribers {
                    topic,
                    except: self.user_id,
                },
                &Event::Typing {
                    conversation_id,
                    order_id,
                    user_id: self.user_id,
                },
            )
            .await;
        Ok(())
    }
}

fn text(reply: &Reply) -> Message {
    // 只含整数、字符串与错误码的结构体序列化不会失败。
    Message::Text(serde_json::to_string(reply).unwrap_or_default().into())
}
