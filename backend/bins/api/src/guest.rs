//! 游客响应缓存（plan 第 8 节「游客页面」）：游客的板块、主题、首页与板块目录响应存 Valkey（[`forum_kv::GuestCache`]），
//! 写操作给对应版本号加一即精确失效；登录用户的请求一律不读也不写这层缓存。
//!
//! 版本号（[`forum_kv::guest_cache`]）与依赖它们的条目：
//!
//! | 条目 | 依赖 |
//! | --- | --- |
//! | 板块目录 `forums` | `forums` |
//! | 板块第 N 页 `board:{id}:{N}`、热门第一页 `board:{id}:hot` | `forums`、`board:{id}` |
//! | 首页最新主题 `latest`、热门第一页 `latest:hot` | `forums`、`latest` |
//! | 主题第 N 页 `topic:{id}:{N}` | `forums`、`topic:{id}` |
//!
//! 名片（用户名、头像、等级）、浏览数、交易帖库存与热门排名不进版本号：新鲜期过后第一个请求回源重建，最多晚 30 秒。

use std::time::Duration;

use axum::Json;
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use forum_core::ErrorCode;
use forum_kv::guest_cache::{self, GuestTtl, Lookup};
use serde::Serialize;

use crate::access::Viewer;
use crate::error::{ApiError, ApiResult};
use crate::posting::after_commit;
use crate::state::AppState;

/// 新鲜 30 秒；过期后 90 秒内，一个请求回源重建时其余请求先拿旧值（版本号没变，只有不进版本号的内容可能旧）。
pub const GUEST_TTL: GuestTtl = GuestTtl {
    fresh: Duration::from_secs(30),
    stale: Duration::from_secs(90),
};
/// 等待别的请求回源的轮数与间隔：最多约 200 ms，之后自己回源（不写缓存）。
const WAIT_ROUNDS: u32 = 20;
const WAIT_STEP: Duration = Duration::from_millis(10);
/// 超过这个大小的响应不缓存（防止个别超大页面挤占 Valkey 内存）。
const MAX_BODY: usize = 1 << 20;

/// 响应头 `x-guest-cache`：`hit` / `stale` / `miss`（本请求回源并写回）/ `bypass`（登录用户或不缓存的请求）。
pub const CACHE_HEADER: &str = "x-guest-cache";

/// 缓存条目：键与它依赖的版本号。`None` 表示这个请求不缓存（例如任意游标的深分页，键空间无界）。
pub struct Entry {
    pub key: String,
    pub versions: Vec<String>,
}

impl Entry {
    pub fn board(forum_id: i32, page: u32) -> Self {
        Self {
            key: format!("board:{forum_id}:{page}"),
            versions: vec![guest_cache::forums(), guest_cache::board(forum_id)],
        }
    }

    /// 板块热门第一页：排名每 5 分钟重算，不进版本号，新鲜期过后回源即可。
    pub fn board_hot(forum_id: i32) -> Self {
        Self {
            key: format!("board:{forum_id}:hot"),
            versions: vec![guest_cache::forums(), guest_cache::board(forum_id)],
        }
    }

    pub fn topic(topic_id: i64, page: i32) -> Self {
        Self {
            key: format!("topic:{topic_id}:{page}"),
            versions: vec![guest_cache::forums(), guest_cache::topic(topic_id)],
        }
    }

    pub fn latest() -> Self {
        Self {
            key: "latest".to_owned(),
            versions: vec![guest_cache::forums(), guest_cache::latest()],
        }
    }

    /// 首页热门第一页（排名同 [`Entry::board_hot`]）。
    pub fn latest_hot() -> Self {
        Self {
            key: "latest:hot".to_owned(),
            versions: vec![guest_cache::forums(), guest_cache::latest()],
        }
    }

    pub fn forums() -> Self {
        Self {
            key: "forums".to_owned(),
            versions: vec![guest_cache::forums()],
        }
    }
}

fn json(body: String, cache: &'static str) -> Response {
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
            (
                header::HeaderName::from_static(CACHE_HEADER),
                HeaderValue::from_static(cache),
            ),
        ],
        body,
    )
        .into_response()
}

fn bypass<T: Serialize>(value: T) -> Response {
    let mut res = Json(value).into_response();
    res.headers_mut().insert(
        header::HeaderName::from_static(CACHE_HEADER),
        HeaderValue::from_static("bypass"),
    );
    res
}

/// 游客且 `entry` 不为空时走缓存，否则直接 `build`。`build` 按同一个访问者组装响应，游客时结果与访问者无关；
/// 返回的 `bool` 为假时这次结果不写缓存（例如超出末页的空页，键空间无界）。
pub async fn respond<T, F, Fut>(
    state: &AppState,
    viewer: &Viewer,
    entry: Option<Entry>,
    build: F,
) -> ApiResult<Response>
where
    T: Serialize,
    F: FnOnce() -> Fut,
    Fut: Future<Output = ApiResult<(T, bool)>>,
{
    let Some(entry) = entry.filter(|_| viewer.user.is_none()) else {
        return Ok(bypass(build().await?.0));
    };
    let cache = &state.guest_cache;
    for _ in 0..WAIT_ROUNDS {
        let found = match cache.lookup(&entry.key, &entry.versions).await {
            Ok(found) => found,
            Err(e) => {
                // 缓存不可用时照常回源，网站不因 Valkey 故障而不可读
                tracing::error!(error = %e, "guest cache lookup failed");
                return Ok(bypass(build().await?.0));
            }
        };
        match found {
            Lookup::Hit(body) => return Ok(json(body, "hit")),
            Lookup::Stale(body) => return Ok(json(body, "stale")),
            Lookup::Wait => tokio::time::sleep(WAIT_STEP).await,
            Lookup::Build(ticket) => {
                let (value, cacheable) = match build().await {
                    Ok(v) => v,
                    Err(e) => {
                        // 内容已不存在或对游客不可见：连同旧条目一起删掉，不再当作过期内容返回
                        let gone = e.0 == ErrorCode::NotFound;
                        after_commit(
                            "guest cache release",
                            cache.release(&entry.key, ticket, gone),
                        )
                        .await;
                        return Err(e);
                    }
                };
                let body = serde_json::to_string(&value).map_err(ApiError::internal)?;
                if cacheable && body.len() <= MAX_BODY {
                    after_commit(
                        "guest cache store",
                        cache.store(&entry.key, ticket, &body, GUEST_TTL),
                    )
                    .await;
                } else {
                    after_commit(
                        "guest cache release",
                        cache.release(&entry.key, ticket, false),
                    )
                    .await;
                }
                return Ok(json(body, "miss"));
            }
        }
    }
    Ok(bypass(build().await?.0))
}

/// 写操作之后：让这些版本号失效（数据库已提交，失败只记日志）。
pub async fn invalidate(state: &AppState, versions: &[String]) {
    after_commit("guest cache bump", state.guest_cache.bump(versions)).await;
}

/// 主题内容变化（回帖、编辑、删除、恢复、点赞、评论）：该主题的各页。
pub async fn topic_changed(state: &AppState, topic_id: i64) {
    invalidate(state, &[guest_cache::topic(topic_id)]).await;
}

/// 主题在列表里的样子变化（新主题、新回复、置顶、精华、锁定、隐藏、移动、改标题、拍卖出价与状态）：该主题、所在板块与首页。
pub async fn listing_changed(state: &AppState, topic_id: i64, forum_ids: &[i32]) {
    invalidate(state, &guest_cache::listing(topic_id, forum_ids)).await;
}

/// 板块本身变化（新建、改名、改可见等级、改颜色）：所有游客条目。后台改板块的接口必须调用。
pub async fn forums_changed(state: &AppState) {
    invalidate(state, &[guest_cache::forums()]).await;
}
