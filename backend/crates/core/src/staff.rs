//! 管理角色与权限（ADR 0013）：站长、管理员、版主三个角色；所有管理接口都经 [`StaffRoles::allows`] 判断。
//!
//! - 版主：只在自己负责的板块内置顶、精华、锁帖、隐藏、移动、删帖（软删除）、禁言（≤ 7 天）、禁拍（≤ 30 天）；
//! - 管理员：全站的用户、板块、内容管理，更长的禁言与禁拍、封号（含永久）、IP / 设备封禁；
//! - 站长：全部权限，另外任免管理员与版主、改阈值、一键撤销、恢复被暂停的管理权限（这几项只在后台做）。
//!
//! 管理权限被自动暂停（异常检测）后论坛内的管理权限全部失效；只在后台用的站长权限不受影响，
//! 因为后台另有 Passkey 登录，站长要靠它恢复（包括恢复自己）。

use serde::{Deserialize, Serialize};

/// `staff_roles.role` 的编号（与数据库一致，仲裁资格按 `role IN (1, 3)` 判断）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Moderator,
    Admin,
    Owner,
}

impl Role {
    pub const fn id(self) -> i16 {
        match self {
            Self::Owner => 1,
            Self::Admin => 3,
            Self::Moderator => 4,
        }
    }

    /// 中文名（告警、邮件等给人看的文字）。
    pub const fn label(self) -> &'static str {
        match self {
            Self::Owner => "站长",
            Self::Admin => "管理员",
            Self::Moderator => "版主",
        }
    }

    pub const fn from_id(id: i16) -> Option<Self> {
        match id {
            1 => Some(Self::Owner),
            3 => Some(Self::Admin),
            4 => Some(Self::Moderator),
            _ => None,
        }
    }
}

/// 管理操作的权限。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    /// 看隐藏主题和等级不够的板块、在锁定主题里回复。
    BoardAccess,
    /// 置顶、精华、锁定、移动（移动要求源板块与目标板块都有权限）。
    TopicModerate,
    /// 隐藏 / 取消隐藏主题。
    TopicHide,
    /// 删除他人楼层（软删除）与恢复。
    PostDelete,
    /// 编辑他人楼层。
    PostEdit,
    Mute,
    AuctionBan,
    /// 封号。
    Ban,
    IpBan,
    DeviceBan,
    /// 查看用户的处罚记录。
    ViewSanctions,
    /// 任免管理员与版主（后台）。
    ManageStaff,
    /// 修改站点参数，如限速与异常阈值（后台）。
    ManageSettings,
    /// 一键撤销某账号某时间段的管理操作（后台）。
    RevokeActions,
    /// 恢复被暂停的管理权限（后台）。
    Reinstate,
}

impl Permission {
    pub const ALL: [Self; 15] = [
        Self::BoardAccess,
        Self::TopicModerate,
        Self::TopicHide,
        Self::PostDelete,
        Self::PostEdit,
        Self::Mute,
        Self::AuctionBan,
        Self::Ban,
        Self::IpBan,
        Self::DeviceBan,
        Self::ViewSanctions,
        Self::ManageStaff,
        Self::ManageSettings,
        Self::RevokeActions,
        Self::Reinstate,
    ];

    /// 版主在自己板块内也有的权限。
    pub const fn moderator(self) -> bool {
        matches!(
            self,
            Self::BoardAccess
                | Self::TopicModerate
                | Self::TopicHide
                | Self::PostDelete
                | Self::Mute
                | Self::AuctionBan
                | Self::ViewSanctions
        )
    }

    /// 只有站长有、只在后台使用的权限。
    pub const fn owner_only(self) -> bool {
        matches!(
            self,
            Self::ManageStaff | Self::ManageSettings | Self::RevokeActions | Self::Reinstate
        )
    }
}

/// 操作的范围：某个板块，或全站（如全站禁言、封号）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Board(i32),
    Site,
}

/// 版主禁言最长 7 天。
pub const MODERATOR_MAX_MUTE_SECS: i64 = 7 * 86_400;
/// 版主禁拍最长 30 天。
pub const MODERATOR_MAX_AUCTION_BAN_SECS: i64 = 30 * 86_400;

/// 处罚时长上限。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurationLimit {
    /// 最长秒数，不能永久。
    Upto(i64),
    /// 不限，可以永久。
    Unlimited,
}

impl DurationLimit {
    /// `secs` 为空表示永久。
    pub fn allows(self, secs: Option<i64>) -> bool {
        match (self, secs) {
            (Self::Unlimited, _) => true,
            (Self::Upto(max), Some(s)) => s <= max,
            (Self::Upto(_), None) => false,
        }
    }
}

/// 一个账号的管理角色。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StaffRoles {
    pub owner: bool,
    pub admin: bool,
    /// 作为版主负责的板块。
    pub boards: Vec<i32>,
    /// 管理权限被自动暂停，待站长恢复。
    pub suspended: bool,
}

impl StaffRoles {
    /// 由 `staff_roles` 的 `(role, forum_id)` 行组装；未知编号忽略。
    pub fn from_rows(rows: impl IntoIterator<Item = (i16, Option<i32>)>, suspended: bool) -> Self {
        let mut s = Self {
            suspended,
            ..Self::default()
        };
        for (role, forum) in rows {
            match (Role::from_id(role), forum) {
                (Some(Role::Owner), _) => s.owner = true,
                (Some(Role::Admin), _) => s.admin = true,
                (Some(Role::Moderator), Some(f)) => s.boards.push(f),
                _ => {}
            }
        }
        s.boards.sort_unstable();
        s.boards.dedup();
        s
    }

    pub fn is_staff(&self) -> bool {
        self.owner || self.admin || !self.boards.is_empty()
    }

    pub fn roles(&self) -> Vec<Role> {
        let mut r = Vec::new();
        if self.owner {
            r.push(Role::Owner);
        }
        if self.admin {
            r.push(Role::Admin);
        }
        if !self.boards.is_empty() {
            r.push(Role::Moderator);
        }
        r
    }

    /// 最高角色；普通用户为 `None`。
    pub fn rank(&self) -> Option<Role> {
        self.roles().into_iter().max()
    }

    /// 能否在 `scope` 内做 `p`。
    pub fn allows(&self, p: Permission, scope: Scope) -> bool {
        if p.owner_only() {
            return self.owner;
        }
        if self.suspended {
            return false;
        }
        if self.owner || self.admin {
            return true;
        }
        match scope {
            Scope::Board(f) => p.moderator() && self.boards.binary_search(&f).is_ok(),
            Scope::Site => false,
        }
    }

    /// 在任一范围内有 `p`（全站，或自己负责的某个板块）。
    pub fn allows_anywhere(&self, p: Permission) -> bool {
        self.allows(p, Scope::Site) || self.boards.iter().any(|b| self.allows(p, Scope::Board(*b)))
    }

    /// 在 `scope` 内做 `p`（禁言 / 禁拍）的时长上限；没有权限返回 `None`。
    pub fn duration_limit(&self, p: Permission, scope: Scope) -> Option<DurationLimit> {
        if !self.allows(p, scope) {
            return None;
        }
        if self.owner || self.admin {
            return Some(DurationLimit::Unlimited);
        }
        match p {
            Permission::Mute => Some(DurationLimit::Upto(MODERATOR_MAX_MUTE_SECS)),
            Permission::AuctionBan => Some(DurationLimit::Upto(MODERATOR_MAX_AUCTION_BAN_SECS)),
            _ => Some(DurationLimit::Unlimited),
        }
    }

    /// 能否处罚 `target`：不能处罚自己，只能处罚角色比自己低的人（版主不能禁言管理员，管理员不能封管理员或站长）。
    pub fn outranks(&self, target: &Self) -> bool {
        match (self.rank(), target.rank()) {
            (None, _) => false,
            (Some(_), None) => true,
            (Some(a), Some(b)) => a > b,
        }
    }

    /// 论坛内可用的全站权限（展示管理功能用）。
    pub fn site_permissions(&self) -> Vec<Permission> {
        Permission::ALL
            .into_iter()
            .filter(|p| !p.owner_only() && self.allows(*p, Scope::Site))
            .collect()
    }

    /// 在自己负责的板块内可用的权限（全站权限之外的部分；管理员与站长为空）。
    pub fn board_permissions(&self) -> Vec<Permission> {
        if self.owner || self.admin || self.suspended {
            return Vec::new();
        }
        Permission::ALL
            .into_iter()
            .filter(|p| p.moderator())
            .collect()
    }
}

/// 站长在后台调整的限速与异常检测阈值（ADR 0013 第 6 条），带默认值与硬上下限。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(default)]
pub struct ModerationSettings {
    /// 限速窗口（秒）。
    pub rate_window_secs: u32,
    /// 每个窗口内最多：置顶 / 精华 / 锁定 / 移动 / 编辑 / 恢复。
    pub rate_content: u32,
    /// 每个窗口内最多：删帖、隐藏主题。
    pub rate_delete: u32,
    /// 每个窗口内最多：禁言、禁拍、封号、IP / 设备封禁及解除。
    pub rate_sanction: u32,
    /// 异常检测窗口（秒）。
    pub anomaly_window_secs: u32,
    /// 窗口内删帖 + 隐藏超过这个数即暂停管理权限并告警。
    pub anomaly_deletes: u32,
    /// 窗口内处罚超过这个数即暂停管理权限并告警。
    pub anomaly_sanctions: u32,
}

impl Default for ModerationSettings {
    fn default() -> Self {
        Self {
            rate_window_secs: 600,
            rate_content: 60,
            rate_delete: 30,
            rate_sanction: 20,
            anomaly_window_secs: 3600,
            anomaly_deletes: 50,
            anomaly_sanctions: 20,
        }
    }
}

impl ModerationSettings {
    /// 每项的 `(下限, 上限)`：太松等于关闭兜底，太紧会误伤正常管理。
    pub const RATE_WINDOW: (u32, u32) = (60, 86_400);
    pub const RATE: (u32, u32) = (1, 1000);
    pub const ANOMALY_WINDOW: (u32, u32) = (300, 86_400);
    pub const ANOMALY_DELETES: (u32, u32) = (5, 1000);
    pub const ANOMALY_SANCTIONS: (u32, u32) = (3, 500);

    /// 每项的最小值与最大值（后台表单用）。
    pub fn bounds() -> (Self, Self) {
        let each = |pick: fn((u32, u32)) -> u32| Self {
            rate_window_secs: pick(Self::RATE_WINDOW),
            rate_content: pick(Self::RATE),
            rate_delete: pick(Self::RATE),
            rate_sanction: pick(Self::RATE),
            anomaly_window_secs: pick(Self::ANOMALY_WINDOW),
            anomaly_deletes: pick(Self::ANOMALY_DELETES),
            anomaly_sanctions: pick(Self::ANOMALY_SANCTIONS),
        };
        (each(|b| b.0), each(|b| b.1))
    }

    fn in_range(v: u32, (lo, hi): (u32, u32)) -> bool {
        (lo..=hi).contains(&v)
    }

    pub fn is_valid(&self) -> bool {
        Self::in_range(self.rate_window_secs, Self::RATE_WINDOW)
            && [self.rate_content, self.rate_delete, self.rate_sanction]
                .into_iter()
                .all(|v| Self::in_range(v, Self::RATE))
            && Self::in_range(self.anomaly_window_secs, Self::ANOMALY_WINDOW)
            && Self::in_range(self.anomaly_deletes, Self::ANOMALY_DELETES)
            && Self::in_range(self.anomaly_sanctions, Self::ANOMALY_SANCTIONS)
    }

    /// 把库里的值收回到上下限内（库里的值只经后台校验后写入，这是兜底）。
    pub fn clamped(self) -> Self {
        let c = |v: u32, (lo, hi): (u32, u32)| v.clamp(lo, hi);
        Self {
            rate_window_secs: c(self.rate_window_secs, Self::RATE_WINDOW),
            rate_content: c(self.rate_content, Self::RATE),
            rate_delete: c(self.rate_delete, Self::RATE),
            rate_sanction: c(self.rate_sanction, Self::RATE),
            anomaly_window_secs: c(self.anomaly_window_secs, Self::ANOMALY_WINDOW),
            anomaly_deletes: c(self.anomaly_deletes, Self::ANOMALY_DELETES),
            anomaly_sanctions: c(self.anomaly_sanctions, Self::ANOMALY_SANCTIONS),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn moderator(boards: &[i32]) -> StaffRoles {
        StaffRoles::from_rows(boards.iter().map(|b| (4, Some(*b))), false)
    }

    fn admin() -> StaffRoles {
        StaffRoles::from_rows([(3, None)], false)
    }

    fn owner() -> StaffRoles {
        StaffRoles::from_rows([(1, None)], false)
    }

    #[test]
    fn moderator_only_in_own_board() {
        let m = moderator(&[7, 3]);
        for p in [
            Permission::BoardAccess,
            Permission::TopicModerate,
            Permission::TopicHide,
            Permission::PostDelete,
            Permission::Mute,
            Permission::AuctionBan,
        ] {
            assert!(m.allows(p, Scope::Board(3)), "{p:?}");
            assert!(m.allows(p, Scope::Board(7)), "{p:?}");
            assert!(!m.allows(p, Scope::Board(4)), "{p:?}");
            assert!(!m.allows(p, Scope::Site), "{p:?}");
        }
        for p in [
            Permission::PostEdit,
            Permission::Ban,
            Permission::IpBan,
            Permission::DeviceBan,
            Permission::ManageStaff,
            Permission::RevokeActions,
        ] {
            assert!(!m.allows(p, Scope::Board(3)), "{p:?}");
        }
    }

    #[test]
    fn moderator_duration_caps() {
        let m = moderator(&[1]);
        let mute = m.duration_limit(Permission::Mute, Scope::Board(1)).unwrap();
        assert!(mute.allows(Some(7 * 86_400)));
        assert!(!mute.allows(Some(8 * 86_400)));
        assert!(!mute.allows(None));
        let ban = m
            .duration_limit(Permission::AuctionBan, Scope::Board(1))
            .unwrap();
        assert!(ban.allows(Some(30 * 86_400)));
        assert!(!ban.allows(Some(30 * 86_400 + 1)));
        assert_eq!(m.duration_limit(Permission::Mute, Scope::Board(2)), None);
        assert_eq!(
            admin().duration_limit(Permission::Mute, Scope::Site),
            Some(DurationLimit::Unlimited)
        );
    }

    #[test]
    fn admin_site_wide_but_not_owner_powers() {
        let a = admin();
        for p in Permission::ALL {
            assert_eq!(a.allows(p, Scope::Site), !p.owner_only(), "{p:?}");
            assert_eq!(a.allows(p, Scope::Board(9)), !p.owner_only(), "{p:?}");
        }
        let o = owner();
        assert!(Permission::ALL.iter().all(|p| o.allows(*p, Scope::Site)));
    }

    #[test]
    fn suspension_removes_forum_powers_but_not_owner_backend() {
        let mut o = owner();
        o.suspended = true;
        assert!(!o.allows(Permission::PostDelete, Scope::Site));
        assert!(o.allows(Permission::Reinstate, Scope::Site));
        let mut m = moderator(&[1]);
        m.suspended = true;
        assert!(!m.allows(Permission::BoardAccess, Scope::Board(1)));
        assert!(m.board_permissions().is_empty());
    }

    #[test]
    fn rank_rules() {
        let user = StaffRoles::default();
        assert!(!user.outranks(&user));
        assert!(moderator(&[1]).outranks(&user));
        assert!(!moderator(&[1]).outranks(&moderator(&[2])));
        assert!(!moderator(&[1]).outranks(&admin()));
        assert!(admin().outranks(&moderator(&[1])));
        assert!(!admin().outranks(&admin()));
        assert!(!admin().outranks(&owner()));
        assert!(owner().outranks(&admin()));
        assert!(!owner().outranks(&owner()));
        // 同时是管理员和版主的人按最高角色算
        let both = StaffRoles::from_rows([(3, None), (4, Some(1))], false);
        assert_eq!(both.rank(), Some(Role::Admin));
    }

    #[test]
    fn rows_ignore_retired_roles() {
        let s = StaffRoles::from_rows([(2, None), (5, None), (4, None)], false);
        assert!(!s.is_staff());
        assert_eq!(Role::from_id(Role::Admin.id()), Some(Role::Admin));
    }

    #[test]
    fn settings_bounds() {
        let d = ModerationSettings::default();
        assert!(d.is_valid());
        assert_eq!(d.clamped(), d);
        let bad = ModerationSettings {
            anomaly_deletes: 0,
            rate_window_secs: 1_000_000,
            ..d
        };
        assert!(!bad.is_valid());
        let (min, max) = ModerationSettings::bounds();
        assert!(min.is_valid() && max.is_valid());
        assert_eq!(min.anomaly_deletes, 5);
        let fixed = bad.clamped();
        assert!(fixed.is_valid());
        assert_eq!(fixed.anomaly_deletes, 5);
        // 缺字段按默认值
        let partial: ModerationSettings = serde_json::from_str(r#"{"rate_delete": 9}"#).unwrap();
        assert_eq!(partial.rate_delete, 9);
        assert_eq!(partial.rate_content, d.rate_content);
    }
}
