//! 访问者与权限：游客等级 0，注册用户按等级；版主、管理员绕过等级限制（plan 第 4、9 节）。

use forum_core::ErrorCode;
use forum_db::forums::Forum;
use forum_db::users::Actor;
use time::{Duration, OffsetDateTime};
use tower_sessions::Session;

use crate::error::{ApiError, ApiResult};
use crate::routes::SESSION_USER_ID;
use crate::state::AppState;

/// 注册多久以内算新用户（plan 第 9 节：24 小时内限发帖频率、不能发外链）。
pub const NEW_USER_PERIOD: Duration = Duration::hours(24);

#[derive(Debug, Clone, Copy)]
pub struct Viewer {
    pub user: Option<Actor>,
}

impl Viewer {
    /// 读会话；未登录或账号已不存在都当游客。
    pub async fn load(state: &AppState, session: &Session) -> ApiResult<Self> {
        let user = match session.get::<i64>(SESSION_USER_ID).await? {
            Some(id) => forum_db::users::actor(&state.db, id).await?,
            None => None,
        };
        Ok(Self { user })
    }

    pub fn level(&self) -> i16 {
        self.user.map_or(0, |u| u.level)
    }

    pub fn require(&self) -> ApiResult<Actor> {
        self.user.ok_or(ApiError(ErrorCode::Unauthorized))
    }

    /// 能否管理该板块。游客直接为否，不查库。
    pub async fn is_staff(&self, state: &AppState, forum_id: i32) -> ApiResult<bool> {
        match self.user {
            Some(u) => Ok(forum_db::staff::can_moderate(&state.db, u.id, forum_id).await?),
            None => Ok(false),
        }
    }

    /// 能否看到该板块。等级够就不查管理角色。
    pub async fn can_view(&self, state: &AppState, forum: &Forum) -> ApiResult<bool> {
        if self.level() >= forum.view_min_level {
            return Ok(true);
        }
        self.is_staff(state, forum.id).await
    }
}

/// 注册不满 24 小时。
pub fn is_new_user(actor: &Actor) -> bool {
    OffsetDateTime::now_utc() - actor.created_at < NEW_USER_PERIOD
}
