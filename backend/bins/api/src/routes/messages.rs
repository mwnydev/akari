//! 私信（plan 第 4、7 节）：一对一会话、消息、已读、屏蔽；担保约定卡片以 `offer` 消息出现在会话里。
//!
//! 只有会话双方能读写，其他人一律 404。消息提交后立即经 Valkey 广播推给双方所有在线设备（`message` 事件），
//! 已读推 `read` 事件。对方此前已读完时另写一条 `notify.message` 发件箱消息（同一段未读只通知一次）。

use std::time::Duration;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use forum_core::ErrorCode;
use forum_core::chat::{Event, MAX_MESSAGE_CHARS, Message, needs_warning};
use forum_db::messages::{self as db, Conversation, NewMessage};
use forum_db::trade::OfferRow;
use forum_db::{Postgres, Transaction, outbox};
use forum_kv::SlidingWindow;
use forum_kv::realtime::Route;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tower_sessions::Session;
use utoipa::{IntoParams, ToSchema};

use super::me::current_user_id;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::posting::limit;
use crate::state::AppState;
use crate::views::UserCardView;

/// 发私信与开新会话的限流（初始值，按运营情况调整）。
const MESSAGE_RATE: SlidingWindow = SlidingWindow::new("dm-send", 60, Duration::from_secs(60));
const CONVERSATION_RATE: SlidingWindow =
    SlidingWindow::new("dm-open", 30, Duration::from_secs(3600));

/// 发件箱：对方有新私信（worker 写成 `message` 通知）。
pub const NOTIFY_MESSAGE: &str = "notify.message";
/// 发件箱：报价卡片事件（worker 写成 `offer` 通知）。
pub const NOTIFY_OFFER: &str = "notify.offer";

#[derive(Debug, Serialize, ToSchema)]
pub struct ConversationView {
    pub id: i64,
    pub peer: UserCardView,
    /// 最后一条消息；新会话还没有消息时为空。
    pub last_message: Option<Message>,
    /// 对方发来、我还没读的消息数（最多 100）。
    pub unread: i64,
    pub my_read_id: i64,
    /// 对方已读到的消息 ID（我发的消息的已读回执）。
    pub peer_read_id: i64,
    /// 我屏蔽了对方。
    pub blocked: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ConversationPage {
    pub conversations: Vec<ConversationView>,
    pub next_cursor: Option<i64>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct CursorQuery {
    /// 上一页的 `next_cursor`。
    pub cursor: Option<i64>,
}

/// 消息翻页：`before` 取它之前的一页、`after` 取它之后的一页（重连补拉），都不给取最新一页；两者不能同时给。
#[derive(Debug, Deserialize, IntoParams)]
pub struct MessagesQuery {
    pub before: Option<i64>,
    pub after: Option<i64>,
}

/// 一页消息，按时间正序。
#[derive(Debug, Serialize, ToSchema)]
pub struct MessagePage {
    pub messages: Vec<Message>,
    /// 翻页方向上还有更多（`after` 时为之后还有，否则为之前还有）。
    pub has_more: bool,
}

/// 翻页方向。
pub enum Direction {
    Before(i64),
    After(i64),
}

impl MessagesQuery {
    pub fn direction(&self) -> ApiResult<Direction> {
        match (self.before, self.after) {
            (Some(_), Some(_)) => Err(ApiError(ErrorCode::InvalidParameter)),
            (_, Some(a)) => Ok(Direction::After(a)),
            (b, None) => Ok(Direction::Before(b.unwrap_or(i64::MAX))),
        }
    }
}

/// 多取一条判断是否还有更多，并统一成时间正序。
pub fn paged<T>(mut rows: Vec<T>, dir: &Direction, limit: i64) -> (Vec<T>, bool) {
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    let has_more = rows.len() > limit;
    rows.truncate(limit);
    if matches!(dir, Direction::Before(_)) {
        rows.reverse();
    }
    (rows, has_more)
}

/// 会话（只有双方可见）。
async fn member_conversation(state: &AppState, id: i64, user_id: i64) -> ApiResult<Conversation> {
    db::conversation(&state.db, id)
        .await?
        .filter(|c| c.is_member(user_id))
        .ok_or(ApiError(ErrorCode::NotFound))
}

async fn peer_card(state: &AppState, peer: i64) -> ApiResult<UserCardView> {
    forum_db::users::cards(&state.db, &[peer])
        .await?
        .pop()
        .map(UserCardView::from)
        .ok_or(ApiError(ErrorCode::NotFound))
}

async fn view(state: &AppState, c: &Conversation, me: i64) -> ApiResult<ConversationView> {
    let peer = c.peer(me);
    let last_message = match c.last_message_id {
        Some(id) => db::by_id(&state.db, id).await?,
        None => None,
    };
    Ok(ConversationView {
        id: c.id,
        peer: peer_card(state, peer).await?,
        last_message,
        unread: db::unread(&state.db, c.id, me, c.read_id(me)).await?,
        my_read_id: c.read_id(me),
        peer_read_id: c.read_id(peer),
        blocked: db::has_blocked(&state.db, me, peer).await?,
    })
}

/// 我的会话列表（有消息的会话，最近的在前，每页 20 个）。
#[utoipa::path(
    get, path = "/api/v1/conversations", tag = "messages", operation_id = "list_conversations",
    params(CursorQuery),
    responses((status = 200, body = ConversationPage), (status = 401, body = ErrorBody))
)]
pub async fn list(
    State(state): State<AppState>,
    session: Session,
    Query(q): Query<CursorQuery>,
) -> ApiResult<Json<ConversationPage>> {
    let me = current_user_id(&session).await?;
    let rows = db::list(
        &state.db,
        me,
        q.cursor.unwrap_or(i64::MAX),
        db::CONVERSATIONS_PAGE,
    )
    .await?;
    let next_cursor = rows
        .last()
        .map(|r| r.last_message.id)
        .filter(|_| i64::try_from(rows.len()) == Ok(db::CONVERSATIONS_PAGE));
    let peers: Vec<i64> = rows.iter().map(|r| r.peer_id).collect();
    let cards = forum_db::users::cards(&state.db, &peers).await?;
    let conversations = rows
        .into_iter()
        .filter_map(|r| {
            let card = cards.iter().find(|c| c.id == r.peer_id)?.clone();
            Some(ConversationView {
                id: r.id,
                peer: card.into(),
                last_message: Some(r.last_message),
                unread: r.unread,
                my_read_id: r.my_read_id,
                peer_read_id: r.peer_read_id,
                blocked: r.blocked,
            })
        })
        .collect();
    Ok(Json(ConversationPage {
        conversations,
        next_cursor,
    }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct OpenConversation {
    pub user_id: i64,
}

/// 打开与某人的会话（已有则返回原会话：200；新建：201）。不能和自己、也不能和互相屏蔽的人开会话。
#[utoipa::path(
    post, path = "/api/v1/conversations", tag = "messages", operation_id = "open_conversation",
    request_body = OpenConversation,
    responses(
        (status = 200, body = ConversationView), (status = 201, body = ConversationView),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody), (status = 429, body = ErrorBody)
    )
)]
pub async fn open(
    State(state): State<AppState>,
    session: Session,
    Json(req): Json<OpenConversation>,
) -> ApiResult<(StatusCode, Json<ConversationView>)> {
    let me = current_user_id(&session).await?;
    if req.user_id == me {
        return Err(ApiError(ErrorCode::InvalidParameter));
    }
    peer_card(&state, req.user_id).await?;
    if db::blocked_between(&state.db, me, req.user_id).await? {
        return Err(ApiError(ErrorCode::UserBlocked));
    }
    // 只有新开会话计入限流，重复打开已有会话不算。
    let (c, created) = match db::between(&state.db, me, req.user_id).await? {
        Some(c) => (c, false),
        None => {
            limit(&state, &CONVERSATION_RATE, me).await?;
            db::get_or_create(&state.db, me, req.user_id).await?
        }
    };
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(view(&state, &c, me).await?)))
}

#[utoipa::path(
    get, path = "/api/v1/conversations/{id}", tag = "messages", operation_id = "get_conversation",
    params(("id" = i64, Path)),
    responses((status = 200, body = ConversationView), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody))
)]
pub async fn get(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
) -> ApiResult<Json<ConversationView>> {
    let me = current_user_id(&session).await?;
    let c = member_conversation(&state, id, me).await?;
    Ok(Json(view(&state, &c, me).await?))
}

/// 会话消息，每页 50 条、按时间正序。
#[utoipa::path(
    get, path = "/api/v1/conversations/{id}/messages", tag = "messages", operation_id = "list_messages",
    params(("id" = i64, Path), MessagesQuery),
    responses(
        (status = 200, body = MessagePage), (status = 400, body = ErrorBody), (status = 401, body = ErrorBody),
        (status = 404, body = ErrorBody)
    )
)]
pub async fn messages(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
    Query(q): Query<MessagesQuery>,
) -> ApiResult<Json<MessagePage>> {
    let me = current_user_id(&session).await?;
    let dir = q.direction()?;
    member_conversation(&state, id, me).await?;
    Ok(Json(page(&state, id, &dir, i64::MAX).await?))
}

/// 一页消息；`upto` 之后的不返回（订单的谈判记录只到付款时）。
pub async fn page(
    state: &AppState,
    conversation_id: i64,
    dir: &Direction,
    upto: i64,
) -> ApiResult<MessagePage> {
    let n = db::MESSAGES_PAGE;
    let mut rows = match *dir {
        Direction::Before(b) => {
            db::before(
                &state.db,
                conversation_id,
                b.min(upto.saturating_add(1)),
                n + 1,
            )
            .await?
        }
        Direction::After(a) => db::after(&state.db, conversation_id, a, n + 1).await?,
    };
    rows.retain(|m| m.id <= upto);
    let (messages, has_more) = paged(rows, dir, n);
    Ok(MessagePage { messages, has_more })
}

/// 文字消息的正文：去掉首尾空白，非空且不超过 2000 字。
pub fn message_body(body: &str) -> ApiResult<&str> {
    let body = body.trim();
    if body.is_empty() {
        return Err(ApiError(ErrorCode::ContentEmpty));
    }
    if body.chars().count() > MAX_MESSAGE_CHARS {
        return Err(ApiError(ErrorCode::ContentTooLong));
    }
    Ok(body)
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SendMessage {
    /// 文字，≤ 2000 字（图片消息在图片上传上线后支持）。
    pub body: String,
}

/// 发私信。互相屏蔽后不能再发。
#[utoipa::path(
    post, path = "/api/v1/conversations/{id}/messages", tag = "messages", operation_id = "send_message",
    params(("id" = i64, Path)),
    request_body = SendMessage,
    responses(
        (status = 201, body = Message), (status = 400, body = ErrorBody), (status = 401, body = ErrorBody),
        (status = 403, body = ErrorBody), (status = 404, body = ErrorBody), (status = 429, body = ErrorBody)
    )
)]
pub async fn send(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
    Json(req): Json<SendMessage>,
) -> ApiResult<(StatusCode, Json<Message>)> {
    let me = current_user_id(&session).await?;
    let body = message_body(&req.body)?;
    let c = member_conversation(&state, id, me).await?;
    let peer = c.peer(me);
    if db::blocked_between(&state.db, me, peer).await? {
        return Err(ApiError(ErrorCode::UserBlocked));
    }
    limit(&state, &MESSAGE_RATE, me).await?;

    let mut tx = state.db.begin().await?;
    let c = db::lock_conversation(&mut tx, id)
        .await?
        .ok_or(ApiError(ErrorCode::NotFound))?;
    let msg = db::insert(
        &mut *tx,
        NewMessage {
            conversation_id: id,
            sender_id: Some(me),
            kind: db::KIND_TEXT,
            body,
            offer_id: None,
            warn: needs_warning(body),
        },
    )
    .await?;
    // 对方此前已读完：这是一段新的未读，通知一次。
    if c.read_id(peer) >= c.last_message_id.unwrap_or(0) {
        outbox::enqueue(
            &mut *tx,
            NOTIFY_MESSAGE,
            &json!({"to": peer, "from": me, "conversation_id": id, "message_id": msg.id}),
            outbox::PRIORITY_NORMAL,
        )
        .await?;
    }
    tx.commit().await?;
    push(&state, &c, &msg).await;
    Ok((StatusCode::CREATED, Json(msg)))
}

/// 推一条新消息给会话双方（事务提交之后调用）。
pub async fn push(state: &AppState, c: &Conversation, msg: &Message) {
    state
        .hub
        .publish(
            Route::Users {
                ids: c.members().to_vec(),
            },
            &Event::Message {
                message: msg.clone(),
            },
        )
        .await;
}

/// 在报价所在会话里写一张卡片消息，并给对方写 `notify.offer`（与报价状态变化同一事务）。
/// 不是私信报价时返回 `None`。`actor` 是这次操作的人（创建、修改、取消为卖家，拒绝为买家）。
pub async fn offer_card(
    tx: &mut Transaction<'_, Postgres>,
    offer: &OfferRow,
    event: &str,
    actor: i64,
) -> ApiResult<Option<(Conversation, Message)>> {
    let Some(conversation_id) = offer.conversation_id else {
        return Ok(None);
    };
    let c = db::lock_conversation(tx, conversation_id)
        .await?
        .ok_or_else(|| ApiError::internal("offer conversation vanished"))?;
    let msg = db::insert(
        &mut **tx,
        NewMessage {
            conversation_id,
            sender_id: Some(actor),
            kind: db::KIND_OFFER,
            body: event,
            offer_id: Some(offer.id),
            warn: false,
        },
    )
    .await?;
    outbox::enqueue(
        &mut **tx,
        NOTIFY_OFFER,
        &json!({
            "to": c.peer(actor), "from": actor, "offer_id": offer.id,
            "conversation_id": conversation_id, "event": event,
        }),
        outbox::PRIORITY_NORMAL,
    )
    .await?;
    Ok(Some((c, msg)))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ReadRequest {
    /// 已读到的消息 ID（含）。
    pub last_id: i64,
}

/// 已读回执：推进我的已读位置（不后退），并推 `read` 事件给双方。
#[utoipa::path(
    post, path = "/api/v1/conversations/{id}/read", tag = "messages", operation_id = "read_conversation",
    params(("id" = i64, Path)),
    request_body = ReadRequest,
    responses((status = 204), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody))
)]
pub async fn read(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
    Json(req): Json<ReadRequest>,
) -> ApiResult<StatusCode> {
    let me = current_user_id(&session).await?;
    let c = member_conversation(&state, id, me).await?;
    if let Some(last_id) = db::mark_read(&state.db, id, me, req.last_id).await? {
        state
            .hub
            .publish(
                Route::Users {
                    ids: c.members().to_vec(),
                },
                &Event::Read {
                    conversation_id: id,
                    user_id: me,
                    last_id,
                },
            )
            .await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// 屏蔽用户：之后双方都不能再给对方发私信（报价卡片等系统消息不受影响）。
#[utoipa::path(
    post, path = "/api/v1/users/{id}/block", tag = "messages", operation_id = "block_user",
    params(("id" = i64, Path)),
    responses(
        (status = 204), (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody)
    )
)]
pub async fn block(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let me = current_user_id(&session).await?;
    if id == me {
        return Err(ApiError(ErrorCode::InvalidParameter));
    }
    peer_card(&state, id).await?;
    db::block(&state.db, me, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    delete, path = "/api/v1/users/{id}/block", tag = "messages", operation_id = "unblock_user",
    params(("id" = i64, Path)),
    responses((status = 204), (status = 401, body = ErrorBody))
)]
pub async fn unblock(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let me = current_user_id(&session).await?;
    db::unblock(&state.db, me, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
