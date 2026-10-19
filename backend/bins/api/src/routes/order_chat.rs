//! 订单内即时通讯（plan 第 7 节）：买卖双方与该订单仲裁案件的仲裁员、站长可读写，其他人 404；记录只追加、带哈希链，仲裁前可校验。
//!
//! 系统消息（付款、发货、开始服务、和解提议、争议、放款……）由 forum-indexer 在订单状态变化的同一事务里写入。
//! 新消息提交后经 Valkey 广播推给双方与订阅了该订单的仲裁员（`order_message` 事件）。

use std::time::Duration;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use forum_core::ErrorCode;
use forum_core::chat::{ChainCheck, Event, OrderMessage, needs_warning};
use forum_db::order_chat::{self as db, NewOrderMessage};
use forum_db::orders::Parties;
use forum_kv::SlidingWindow;
use forum_kv::realtime::Route;
use serde::Serialize;
use tower_sessions::Session;
use utoipa::ToSchema;

use super::me::current_user_id;
use super::messages::{
    Direction, MessagePage, MessagesQuery, ReadRequest, SendMessage, message_body, paged,
};
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::posting::limit;
use crate::state::AppState;
use crate::views::hex0x;

const ORDER_CHAT_RATE: SlidingWindow =
    SlidingWindow::new("order-chat", 60, Duration::from_secs(60));

/// 订单的买卖双方，或该订单仲裁案件的站长与在岗仲裁员；其他人当作订单不存在。
pub async fn accessible(state: &AppState, order_id: i64, user_id: i64) -> ApiResult<Parties> {
    let not_found = || ApiError(ErrorCode::NotFound);
    let p = forum_db::orders::parties(&state.db, order_id)
        .await?
        .ok_or_else(not_found)?;
    if p.is_party(user_id)
        || forum_db::arbitration::can_review_order(&state.db, order_id, user_id).await?
    {
        Ok(p)
    } else {
        Err(not_found())
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ReadMark {
    pub user_id: i64,
    pub last_id: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OrderMessagePage {
    /// 按时间正序。
    pub messages: Vec<OrderMessage>,
    pub has_more: bool,
    /// 各人（买卖双方、仲裁员）的已读位置。
    pub reads: Vec<ReadMark>,
}

/// 订单聊天，每页 50 条、按时间正序（翻页同私信：`before` / `after`，都不给取最新一页）。
#[utoipa::path(
    get, path = "/api/v1/orders/{id}/messages", tag = "orders", operation_id = "list_order_messages",
    params(("id" = i64, Path), MessagesQuery),
    responses(
        (status = 200, body = OrderMessagePage), (status = 400, body = ErrorBody), (status = 401, body = ErrorBody),
        (status = 404, body = ErrorBody)
    )
)]
pub async fn list(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
    Query(q): Query<MessagesQuery>,
) -> ApiResult<Json<OrderMessagePage>> {
    let me = current_user_id(&session).await?;
    let dir = q.direction()?;
    accessible(&state, id, me).await?;
    let rows = match dir {
        Direction::Before(b) => db::before(&state.db, id, b, db::PAGE + 1).await?,
        Direction::After(a) => db::after(&state.db, id, a, db::PAGE + 1).await?,
    };
    let (messages, has_more) = paged(rows, &dir, db::PAGE);
    let reads = db::reads(&state.db, id)
        .await?
        .into_iter()
        .map(|(user_id, last_id)| ReadMark { user_id, last_id })
        .collect();
    Ok(Json(OrderMessagePage {
        messages,
        has_more,
        reads,
    }))
}

/// 在订单聊天里发言（买卖双方与仲裁员）。含钱包地址或「线下转账」等说法时 `warn` 为真，界面提示只走担保。
#[utoipa::path(
    post, path = "/api/v1/orders/{id}/messages", tag = "orders", operation_id = "send_order_message",
    params(("id" = i64, Path)),
    request_body = SendMessage,
    responses(
        (status = 201, body = OrderMessage), (status = 400, body = ErrorBody), (status = 401, body = ErrorBody),
        (status = 404, body = ErrorBody), (status = 429, body = ErrorBody)
    )
)]
pub async fn send(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
    Json(req): Json<SendMessage>,
) -> ApiResult<(StatusCode, Json<OrderMessage>)> {
    let me = current_user_id(&session).await?;
    let body = message_body(&req.body)?;
    let parties = accessible(&state, id, me).await?;
    limit(&state, &ORDER_CHAT_RATE, me).await?;
    let mut tx = state.db.begin().await?;
    let msg = db::append(
        &mut tx,
        NewOrderMessage {
            order_id: id,
            sender_id: Some(me),
            kind: db::KIND_TEXT,
            body,
            warn: needs_warning(body),
        },
    )
    .await?;
    tx.commit().await?;
    state
        .hub
        .publish(
            Route::Order {
                id,
                users: parties.users(),
            },
            &Event::OrderMessage {
                message: msg.clone(),
            },
        )
        .await;
    Ok((StatusCode::CREATED, Json(msg)))
}

/// 订单聊天已读（不后退），推 `order_read` 事件。
#[utoipa::path(
    post, path = "/api/v1/orders/{id}/messages/read", tag = "orders", operation_id = "read_order_messages",
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
    let parties = accessible(&state, id, me).await?;
    if let Some(last_id) = db::mark_read(&state.db, id, me, req.last_id).await? {
        state
            .hub
            .publish(
                Route::Order {
                    id,
                    users: parties.users(),
                },
                &Event::OrderRead {
                    order_id: id,
                    user_id: me,
                    last_id,
                },
            )
            .await;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ChainVerification {
    /// 整条链完好。
    pub ok: bool,
    pub count: u64,
    /// 链头哈希（最后一条消息的 `hash`；空链为 32 个 0 字节）。
    pub head_hash: String,
    /// 第一条对不上的消息 ID（内容被改、被删或被插入）。
    pub broken_at: Option<i64>,
}

impl ChainVerification {
    pub fn of(check: &ChainCheck) -> Self {
        Self {
            ok: check.broken_at.is_none(),
            count: check.count,
            head_hash: hex0x(&check.head()),
            broken_at: check.broken_at,
        }
    }
}

/// 重算订单聊天的哈希链（仲裁员裁决前校验，双方也可自查）。
#[utoipa::path(
    get, path = "/api/v1/orders/{id}/messages/verify", tag = "orders", operation_id = "verify_order_messages",
    params(("id" = i64, Path)),
    responses((status = 200, body = ChainVerification), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody))
)]
pub async fn verify(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
) -> ApiResult<Json<ChainVerification>> {
    let me = current_user_id(&session).await?;
    accessible(&state, id, me).await?;
    Ok(Json(ChainVerification::of(
        &db::verify_chain(&state.db, id).await?,
    )))
}

/// 建单前的私信谈判记录（仲裁证据）：生成该订单的报价所在会话、截至付款上链时的消息。
/// 翻页同私信消息；不是私信报价生成的订单返回 404。
#[utoipa::path(
    get, path = "/api/v1/orders/{id}/negotiation", tag = "orders", operation_id = "order_negotiation",
    params(("id" = i64, Path), MessagesQuery),
    responses(
        (status = 200, body = MessagePage), (status = 400, body = ErrorBody), (status = 401, body = ErrorBody),
        (status = 404, body = ErrorBody)
    )
)]
pub async fn negotiation(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
    Query(q): Query<MessagesQuery>,
) -> ApiResult<Json<MessagePage>> {
    let me = current_user_id(&session).await?;
    let dir = q.direction()?;
    accessible(&state, id, me).await?;
    let n = db::negotiation(&state.db, id)
        .await?
        .ok_or(ApiError(ErrorCode::NotFound))?;
    Ok(Json(
        super::messages::page(&state, n.conversation_id, &dir, n.last_message_id).await?,
    ))
}
