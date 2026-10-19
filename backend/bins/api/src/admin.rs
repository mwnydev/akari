//! 站长后台会话（ADR 0013 第 4 条、ADR 0012）：后台 Passkey（RP ID 为后台中心域名）登录后发放独立的 Cookie。
//!
//! 安全决定：
//! - 后台接口全部在 `/api/v1/admin/` 下，每个请求（含 GET）都必须带后台页面的 `Origin`；论坛页面即使被 XSS，
//!   同源请求也带不上这个 Origin（浏览器禁止脚本改写），所以用不了后台会话。CORS 也只对后台来源开放。
//! - 后台 Cookie 与论坛会话分开：随机 256 位令牌，Valkey 只存其 SHA-256；`HttpOnly`、`SameSite=Strict`、
//!   路径限定 `/api/v1/admin`，**永远不设 Domain**（host-only）。
//! - 闲置 2 小时、最长 12 小时；任免、改阈值、一键撤销、恢复权限等敏感操作要求 5 分钟内用后台 Passkey 再验证过。
//! - 每个请求都重新确认账号仍是站长；被撤销站长角色立即失效。

use std::time::Duration;

use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use forum_auth::passkey::UserHandles;
use forum_auth::passkey::webauthn::Webauthn;
use forum_core::ErrorCode;
use forum_core::staff::{Permission, Scope, StaffRoles};
use forum_kv::ephemeral;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// 后台接口前缀（Origin 校验与 CORS 按它分流）。
pub const ADMIN_PREFIX: &str = "/api/v1/admin";
/// 后台会话 Cookie 名。
pub const ADMIN_COOKIE: &str = "asid";
/// 闲置 2 小时过期、最长 12 小时（plan 第 9 节「管理人员会话」）。
pub const IDLE_SECS: i64 = 2 * 3600;
pub const ABSOLUTE_SECS: i64 = 12 * 3600;
/// 敏感操作再验证有效 5 分钟。
pub const REVERIFY_SECS: i64 = 300;

pub struct AdminState {
    pub webauthn: Webauthn,
    /// 后台页面的 Origin（与请求头逐字比较）。
    pub origin: String,
    pub handles: UserHandles,
}

/// 是否后台接口。
pub fn is_admin_path(path: &str) -> bool {
    path.strip_prefix(ADMIN_PREFIX)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

fn now() -> i64 {
    OffsetDateTime::now_utc().unix_timestamp()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Stored {
    user_id: i64,
    created_at: i64,
    last_seen_at: i64,
    reverified_at: i64,
}

/// 已登录的后台会话；作为提取器使用时已确认账号仍是站长。
#[derive(Debug, Clone)]
pub struct AdminSession {
    pub user_id: i64,
    pub roles: StaffRoles,
    key: String,
    stored: Stored,
}

fn session_key(state: &AppState, token: &str) -> String {
    format!(
        "{}admin:sess:{}",
        state.cache.ns(),
        hex::encode(Sha256::digest(token.as_bytes()))
    )
}

async fn save(state: &AppState, key: &str, s: &Stored) -> ApiResult<()> {
    let remaining = (s.created_at + ABSOLUTE_SECS - now()).min(IDLE_SECS);
    let ttl = u64::try_from(remaining).unwrap_or(0).max(1);
    let json = serde_json::to_string(s).map_err(ApiError::internal)?;
    ephemeral::put(&state.kv, key, &json, Duration::from_secs(ttl)).await?;
    Ok(())
}

impl AdminSession {
    /// 新建会话（刚用后台 Passkey 验证过，同时算一次再验证），返回带 Cookie 的 jar。
    pub async fn create(
        state: &AppState,
        jar: CookieJar,
        user_id: i64,
    ) -> ApiResult<(CookieJar, Self)> {
        // 换新会话前作废这个浏览器上的旧会话。
        if let Some(old) = jar.get(ADMIN_COOKIE) {
            ephemeral::take(&state.kv, &session_key(state, old.value())).await?;
        }
        let token = hex::encode(rand::rng().random::<[u8; 32]>());
        let t = now();
        let stored = Stored {
            user_id,
            created_at: t,
            last_seen_at: t,
            reverified_at: t,
        };
        let key = session_key(state, &token);
        save(state, &key, &stored).await?;
        let roles = forum_db::staff::roles(&state.db, user_id).await?;
        let cookie = Cookie::build((ADMIN_COOKIE, token))
            .path(ADMIN_PREFIX)
            .http_only(true)
            .secure(state.cookie_secure)
            .same_site(SameSite::Strict)
            .max_age(time::Duration::seconds(ABSOLUTE_SECS))
            .build();
        Ok((
            jar.add(cookie),
            Self {
                user_id,
                roles,
                key,
                stored,
            },
        ))
    }

    /// 读 Cookie 对应的会话：不存在、闲置超时、超过最长时长或已不是站长时返回 `None`（并删掉会话）。
    pub async fn load(state: &AppState, jar: &CookieJar) -> ApiResult<Option<Self>> {
        let Some(token) = jar
            .get(ADMIN_COOKIE)
            .map(|c| c.value())
            .filter(|v| v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()))
        else {
            return Ok(None);
        };
        let key = session_key(state, token);
        let Some(raw) = ephemeral::get(&state.kv, &key).await? else {
            return Ok(None);
        };
        let mut stored: Stored = serde_json::from_str(&raw).map_err(ApiError::internal)?;
        let t = now();
        let roles = forum_db::staff::roles(&state.db, stored.user_id).await?;
        if t - stored.last_seen_at > IDLE_SECS
            || t - stored.created_at > ABSOLUTE_SECS
            || !roles.owner
        {
            ephemeral::take(&state.kv, &key).await?;
            return Ok(None);
        }
        stored.last_seen_at = t;
        save(state, &key, &stored).await?;
        Ok(Some(Self {
            user_id: stored.user_id,
            roles,
            key,
            stored,
        }))
    }

    /// 核对后台权限（这些权限只有站长有，见 `forum_core::staff`）。
    pub fn require(&self, p: Permission) -> ApiResult<()> {
        if self.roles.allows(p, Scope::Site) {
            Ok(())
        } else {
            Err(ApiError(ErrorCode::Forbidden))
        }
    }

    /// 敏感操作：5 分钟内用后台 Passkey 验证过。
    pub fn require_recent(&self) -> ApiResult<()> {
        if now() - self.stored.reverified_at <= REVERIFY_SECS {
            Ok(())
        } else {
            Err(ApiError(ErrorCode::AdminReverifyRequired))
        }
    }

    pub async fn mark_reverified(&mut self, state: &AppState) -> ApiResult<()> {
        self.stored.reverified_at = now();
        save(state, &self.key, &self.stored).await
    }

    /// 退出：删除服务端会话并让浏览器删掉 Cookie。
    pub async fn destroy(self, state: &AppState, jar: CookieJar) -> ApiResult<CookieJar> {
        ephemeral::take(&state.kv, &self.key).await?;
        Ok(jar.remove(Cookie::build(ADMIN_COOKIE).path(ADMIN_PREFIX)))
    }

    pub fn expires_at(&self) -> i64 {
        (self.stored.created_at + ABSOLUTE_SECS).min(self.stored.last_seen_at + IDLE_SECS)
    }

    pub fn absolute_expires_at(&self) -> i64 {
        self.stored.created_at + ABSOLUTE_SECS
    }

    /// 再验证有效期截止（已过期为 `None`）。
    pub fn reverified_until(&self) -> Option<i64> {
        Some(self.stored.reverified_at + REVERIFY_SECS).filter(|&u| u >= now())
    }
}

impl<S> FromRequestParts<S> for AdminSession
where
    S: Send + Sync,
    AppState: FromRef<S>,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = AppState::from_ref(state);
        let jar = CookieJar::from_headers(&parts.headers);
        Self::load(&state, &jar)
            .await?
            .ok_or(ApiError(ErrorCode::Unauthorized))
    }
}
