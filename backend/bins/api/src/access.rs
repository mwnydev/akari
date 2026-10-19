//! 访问者与权限：游客等级 0，注册用户按等级；有板块访问权限的管理人员绕过等级与隐藏限制（plan 第 4 节、ADR 0013）。

use forum_core::ErrorCode;
use forum_core::staff::{Permission, Scope, StaffRoles};
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

    /// 管理角色；游客直接为空，不查库。
    pub async fn staff(&self, state: &AppState) -> ApiResult<StaffRoles> {
        match self.user {
            Some(u) => Ok(forum_db::staff::roles(&state.db, u.id).await?),
            None => Ok(StaffRoles::default()),
        }
    }

    /// 能否以管理人员身份进入该板块：看隐藏主题和等级不够的板块、在锁定主题里回复（[`Permission::BoardAccess`]）。
    pub async fn is_staff(&self, state: &AppState, forum_id: i32) -> ApiResult<bool> {
        Ok(self
            .staff(state)
            .await?
            .allows(Permission::BoardAccess, Scope::Board(forum_id)))
    }

    /// 能否看到该板块。等级够就不查管理角色。
    pub async fn can_view(&self, state: &AppState, forum: &Forum) -> ApiResult<bool> {
        if self.level() >= forum.view_min_level {
            return Ok(true);
        }
        self.is_staff(state, forum.id).await
    }
}

/// 访问者能看到的全部板块：等级够的，加上自己管理的（管理范围一次查出，不逐个板块查）。
pub async fn visible_forums(state: &AppState, viewer: &Viewer) -> ApiResult<Vec<Forum>> {
    let all = forum_db::forums::list_all(&state.db).await?;
    let level = viewer.level();
    let roles = if all.iter().any(|f| f.view_min_level > level) {
        viewer.staff(state).await?
    } else {
        StaffRoles::default()
    };
    Ok(all
        .into_iter()
        .filter(|f| {
            f.view_min_level <= level || roles.allows(Permission::BoardAccess, Scope::Board(f.id))
        })
        .collect())
}

/// 注册不满 24 小时。
pub fn is_new_user(actor: &Actor) -> bool {
    OffsetDateTime::now_utc() - actor.created_at < NEW_USER_PERIOD
}
