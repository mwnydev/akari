//! 站长后台的板块管理（ADR 0013）：改名称、简介、色块、排序与能否发交易帖、拍卖帖。只有站长（后台会话），
//! 改「能否发交易帖 / 拍卖帖」涉及钱，另需 5 分钟内用后台 Passkey 再验证；每次修改写审计日志并推送管理员群。
//! 板块列表用 `GET /api/v1/admin/forums`（`routes::admin::admin_forums`）。

use axum::Json;
use axum::extract::{Path, State};
use forum_core::ErrorCode;
use forum_core::staff::Permission;
use forum_db::forums::{self, Forum, ForumPatch, PALETTE};
use forum_db::{audit, outbox};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value, json};
use utoipa::ToSchema;

use crate::admin::AdminSession;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::guest;
use crate::state::AppState;
use crate::views::ForumView;

/// 板块名 1–20 字、简介最多 200 字、排序 0–9999。
const MAX_NAME_CHARS: usize = 20;
const MAX_DESCRIPTION_CHARS: usize = 200;
const MAX_POSITION: i32 = 9999;

/// 区分「没给这个字段」（外层 `None`，不改）与「给了 null」（`Some(None)`，清除）。
fn present<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ForumUpdateRequest {
    /// 1–20 字。
    pub name: Option<String>,
    /// 最多 200 字，可以为空字符串。
    pub description: Option<String>,
    /// 色板里的 `#rrggbb`；`null` 清除（前端按排序位置轮换）；不传不改。
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    pub color: Option<Option<String>>,
    /// 同级板块里的排序，小的在前（0–9999）。
    pub position: Option<i32>,
    /// 能否发交易帖；修改需要 5 分钟内再验证过。
    pub allow_trade: Option<bool>,
    /// 能否发拍卖帖（拍卖区）；修改需要 5 分钟内再验证过。
    pub allow_auction: Option<bool>,
}

fn trimmed(raw: &str, min: usize, max: usize) -> ApiResult<&str> {
    let t = raw.trim();
    let n = t.chars().count();
    if n < min || n > max || t.chars().any(char::is_control) {
        return Err(ApiError(ErrorCode::InvalidParameter));
    }
    Ok(t)
}

/// 校验请求并换成数据库的改动；什么都没改时 `INVALID_PARAMETER`。
fn patch(req: &ForumUpdateRequest) -> ApiResult<ForumPatch<'_>> {
    let color = match &req.color {
        Some(Some(c)) if !PALETTE.contains(&c.as_str()) => {
            return Err(ApiError(ErrorCode::InvalidParameter));
        }
        c => c.as_ref().map(Option::as_deref),
    };
    if req
        .position
        .is_some_and(|p| !(0..=MAX_POSITION).contains(&p))
    {
        return Err(ApiError(ErrorCode::InvalidParameter));
    }
    let p = ForumPatch {
        name: req
            .name
            .as_deref()
            .map(|n| trimmed(n, 1, MAX_NAME_CHARS))
            .transpose()?,
        description: req
            .description
            .as_deref()
            .map(|d| trimmed(d, 0, MAX_DESCRIPTION_CHARS))
            .transpose()?,
        color,
        position: req.position,
        allow_trade: req.allow_trade,
        allow_auction: req.allow_auction,
    };
    if p.name.is_none()
        && p.description.is_none()
        && p.color.is_none()
        && p.position.is_none()
        && p.allow_trade.is_none()
        && p.allow_auction.is_none()
    {
        return Err(ApiError(ErrorCode::InvalidParameter));
    }
    Ok(p)
}

/// 改动前后不同的字段（审计日志）。
fn changes(old: &Forum, new: &Forum) -> (Value, Value) {
    let (mut before, mut after) = (Map::new(), Map::new());
    let mut diff = |k: &str, a: Value, b: Value| {
        if a != b {
            before.insert(k.to_owned(), a);
            after.insert(k.to_owned(), b);
        }
    };
    diff("name", json!(old.name), json!(new.name));
    diff(
        "description",
        json!(old.description),
        json!(new.description),
    );
    diff("color", json!(old.color), json!(new.color));
    diff("position", json!(old.position), json!(new.position));
    diff(
        "allow_trade",
        json!(old.allow_trade),
        json!(new.allow_trade),
    );
    diff(
        "allow_auction",
        json!(old.allow_auction),
        json!(new.allow_auction),
    );
    (Value::Object(before), Value::Object(after))
}

/// 修改板块。论坛里的板块列表与游客页面立即生效；登录用户的主题第 1 页缓存里的板块名与色块最长 10 分钟后更新。
#[utoipa::path(
    patch, path = "/api/v1/admin/forums/{id}", tag = "admin", operation_id = "admin_update_forum",
    params(("id" = i32, Path)),
    request_body = ForumUpdateRequest,
    responses(
        (status = 200, body = ForumView),
        (status = 400, body = ErrorBody), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody)
    )
)]
pub async fn admin_update_forum(
    State(state): State<AppState>,
    admin: AdminSession,
    Path(id): Path<i32>,
    Json(req): Json<ForumUpdateRequest>,
) -> ApiResult<Json<ForumView>> {
    admin.require(Permission::ManageSettings)?;
    let p = patch(&req)?;
    let mut tx = state.db.begin().await?;
    let old = forums::by_id(&mut *tx, id)
        .await?
        .ok_or(ApiError(ErrorCode::NotFound))?;
    if p.allow_trade.is_some_and(|t| t != old.allow_trade)
        || p.allow_auction.is_some_and(|a| a != old.allow_auction)
    {
        admin.require_recent()?;
    }
    let new = forums::update(&mut *tx, id, &p)
        .await?
        .ok_or(ApiError(ErrorCode::NotFound))?;
    let (before, after) = changes(&old, &new);
    if after.as_object().is_some_and(|m| !m.is_empty()) {
        audit::append(
            &mut *tx,
            admin.user_id,
            "forum.update",
            Some(&format!("forum:{id}")),
            None,
            &json!({ "old": before, "new": after }),
        )
        .await?;
        outbox::staff_alert(
            &mut *tx,
            "forum.update",
            &format!("站长修改了板块 #{id}（{}）：{after}", new.name),
        )
        .await?;
    }
    tx.commit().await?;
    // 板块目录、名称、颜色、可见等级出现在游客缓存的每一类页面里
    guest::forums_changed(&state).await;
    Ok(Json((&new).into()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn req(v: Value) -> ForumUpdateRequest {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn color_is_tri_state_and_from_the_palette() {
        let keep = req(json!({ "name": "x" }));
        assert_eq!(patch(&keep).unwrap().color, None);
        let clear = req(json!({ "color": null }));
        assert_eq!(patch(&clear).unwrap().color, Some(None));
        let set = req(json!({ "color": "#2e9d5c" }));
        assert_eq!(patch(&set).unwrap().color, Some(Some("#2e9d5c")));
        assert!(patch(&req(json!({ "color": "#123456" }))).is_err());
    }

    #[test]
    fn fields_are_validated() {
        assert!(patch(&req(json!({}))).is_err());
        assert!(patch(&req(json!({ "name": "  " }))).is_err());
        assert!(patch(&req(json!({ "name": "一".repeat(21) }))).is_err());
        assert_eq!(
            patch(&req(json!({ "name": " 交易区 " }))).unwrap().name,
            Some("交易区")
        );
        assert!(patch(&req(json!({ "description": "" }))).is_ok());
        assert!(patch(&req(json!({ "position": -1 }))).is_err());
        assert!(patch(&req(json!({ "position": 10_000 }))).is_err());
        assert!(serde_json::from_value::<ForumUpdateRequest>(json!({ "slug": "x" })).is_err());
        assert_eq!(
            patch(&req(json!({ "allow_auction": true })))
                .unwrap()
                .allow_auction,
            Some(true)
        );
    }
}
