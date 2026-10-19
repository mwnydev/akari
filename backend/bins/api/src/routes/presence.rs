//! 在线状态（plan 第 7 节「在线状态」）：有 WebSocket 连接即在线（Valkey 连接登记，90 秒没心跳过期），
//! 不在线显示最近活跃时间（`users.last_seen_at`，每分钟最多写一次）。
//! 隐身（`show_online = false`）的人只显示活跃到天，但本人和进行中订单的交易对手仍看到真实状态。

use axum::Json;
use axum::extract::{Query, State};
use forum_core::ErrorCode;
use forum_core::chat::Presence;
use serde::Deserialize;
use tower_sessions::Session;
use utoipa::IntoParams;

use super::SESSION_USER_ID;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::state::AppState;

/// 一次最多查询的人数（列表页一页的作者）。
const MAX_IDS: usize = 100;

/// 按访问者可见性批量取在线状态，顺序同 `ids`，不存在的用户跳过。
pub async fn lookup(
    state: &AppState,
    viewer: Option<i64>,
    ids: &[i64],
) -> ApiResult<Vec<Presence>> {
    let rows = forum_db::users::visibility(&state.db, ids).await?;
    let hidden: Vec<i64> = rows
        .iter()
        .filter(|r| !r.show_online && Some(r.id) != viewer)
        .map(|r| r.id)
        .collect();
    let counterparties = match viewer {
        Some(v) if !hidden.is_empty() => {
            forum_db::orders::open_counterparties(&state.db, v, &hidden).await?
        }
        _ => Vec::new(),
    };
    let online = state.hub.realtime().online(ids).await?;
    Ok(ids
        .iter()
        .zip(online)
        .filter_map(|(id, online)| {
            let r = rows.iter().find(|r| r.id == *id)?;
            let real = !hidden.contains(id) || counterparties.contains(id);
            Some(Presence::of(*id, online, r.last_seen_at, real))
        })
        .collect())
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct PresenceQuery {
    /// 逗号分隔的用户 ID，最多 100 个。
    pub ids: String,
}

/// 批量查询在线状态（列表页停留时每 60 秒刷新一次；私信与订单聊天里另有实时推送）。游客也可查询。
#[utoipa::path(
    get, path = "/api/v1/users/presence", tag = "messages", operation_id = "user_presence",
    params(PresenceQuery),
    responses((status = 200, body = Vec<Presence>), (status = 400, body = ErrorBody))
)]
pub async fn presence(
    State(state): State<AppState>,
    session: Session,
    Query(q): Query<PresenceQuery>,
) -> ApiResult<Json<Vec<Presence>>> {
    let bad = || ApiError(ErrorCode::InvalidParameter);
    let mut ids = q
        .ids
        .split(',')
        .map(|s| s.trim().parse::<i64>().map_err(|_| bad()))
        .collect::<ApiResult<Vec<_>>>()?;
    let mut seen = std::collections::HashSet::new();
    ids.retain(|id| seen.insert(*id));
    if ids.is_empty() || ids.len() > MAX_IDS {
        return Err(bad());
    }
    let viewer = session.get::<i64>(SESSION_USER_ID).await?;
    Ok(Json(lookup(&state, viewer, &ids).await?))
}
