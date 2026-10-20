//! 处罚（ADR 0013）：禁言、禁拍、封号、IP / 设备封禁。到期自动失效；撤销只做标记，可以再恢复。

use sqlx::PgExecutor;
use time::OffsetDateTime;

pub const KIND_MUTE: i16 = 1;
pub const KIND_AUCTION_BAN: i16 = 2;
pub const KIND_BAN: i16 = 3;
pub const KIND_IP_BAN: i16 = 4;
pub const KIND_DEVICE_BAN: i16 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sanction {
    pub id: i64,
    pub kind: i16,
    pub user_id: Option<i64>,
    pub forum_id: Option<i32>,
    /// CIDR 文本，如 `203.0.113.0/24`。
    pub ip: Option<String>,
    pub ends_at: Option<OffsetDateTime>,
    pub reason: String,
    /// 处罚人；系统自动的处罚（拍卖违约禁拍）为空。
    pub created_by: Option<i64>,
    pub created_at: OffsetDateTime,
    pub revoked_at: Option<OffsetDateTime>,
    pub revoked_by: Option<i64>,
}

impl Sanction {
    pub fn active(&self, now: OffsetDateTime) -> bool {
        self.revoked_at.is_none() && self.ends_at.is_none_or(|e| e > now)
    }
}

/// 新处罚。IP 封禁用 [`create_ip_ban`]，设备封禁用 [`create_device_bans`]。
#[derive(Debug, Clone, Copy)]
pub struct NewSanction<'a> {
    pub kind: i16,
    pub user_id: i64,
    pub forum_id: Option<i32>,
    pub ends_at: Option<OffsetDateTime>,
    pub reason: &'a str,
    /// 处罚人；系统自动的处罚为空。
    pub created_by: Option<i64>,
}

pub async fn create(db: impl PgExecutor<'_>, s: NewSanction<'_>) -> Result<i64, sqlx::Error> {
    let row = sqlx::query!(
        "INSERT INTO sanctions (kind, user_id, forum_id, ends_at, reason, created_by)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
        s.kind,
        s.user_id,
        s.forum_id,
        s.ends_at,
        s.reason,
        s.created_by
    )
    .fetch_one(db)
    .await?;
    Ok(row.id)
}

/// 封禁一个 IP 或网段（`cidr` 已校验，如 `203.0.113.7` 或 `2001:db8::/48`）。
pub async fn create_ip_ban(
    db: impl PgExecutor<'_>,
    cidr: &str,
    ends_at: Option<OffsetDateTime>,
    reason: &str,
    created_by: i64,
) -> Result<i64, sqlx::Error> {
    let row = sqlx::query!(
        "INSERT INTO sanctions (kind, ip, ends_at, reason, created_by)
         VALUES ($1, $2::text::cidr, $3, $4, $5) RETURNING id",
        KIND_IP_BAN,
        cidr,
        ends_at,
        reason,
        created_by
    )
    .fetch_one(db)
    .await?;
    Ok(row.id)
}

/// 封禁某用户登录过的全部设备，返回新处罚的 ID（用户没有登录过的设备时为空）。
pub async fn create_device_bans(
    db: impl PgExecutor<'_>,
    user_id: i64,
    ends_at: Option<OffsetDateTime>,
    reason: &str,
    created_by: i64,
) -> Result<Vec<i64>, sqlx::Error> {
    let rows = sqlx::query!(
        "INSERT INTO sanctions (kind, user_id, device_hash, ends_at, reason, created_by)
         SELECT $1, $2, device_hash, $3, $4, $5 FROM user_devices WHERE user_id = $2
         RETURNING id",
        KIND_DEVICE_BAN,
        user_id,
        ends_at,
        reason,
        created_by
    )
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(|r| r.id).collect())
}

pub async fn by_id(db: impl PgExecutor<'_>, id: i64) -> Result<Option<Sanction>, sqlx::Error> {
    sqlx::query_as!(
        Sanction,
        "SELECT id, kind, user_id, forum_id, ip::text AS ip, ends_at, reason, created_by, created_at,
                revoked_at, revoked_by
         FROM sanctions WHERE id = $1",
        id
    )
    .fetch_optional(db)
    .await
}

/// 用户的处罚记录，新的在前（最多 `limit` 条）。
pub async fn of_user(
    db: impl PgExecutor<'_>,
    user_id: i64,
    limit: i64,
) -> Result<Vec<Sanction>, sqlx::Error> {
    sqlx::query_as!(
        Sanction,
        "SELECT id, kind, user_id, forum_id, ip::text AS ip, ends_at, reason, created_by, created_at,
                revoked_at, revoked_by
         FROM sanctions WHERE user_id = $1 ORDER BY id DESC LIMIT $2",
        user_id,
        limit
    )
    .fetch_all(db)
    .await
}

/// 撤销仍在生效的处罚，返回实际撤销的 ID。
pub async fn revoke(
    db: impl PgExecutor<'_>,
    ids: &[i64],
    by: i64,
) -> Result<Vec<i64>, sqlx::Error> {
    let rows = sqlx::query!(
        "UPDATE sanctions SET revoked_at = now(), revoked_by = $2
         WHERE id = ANY($1::bigint[]) AND revoked_at IS NULL AND (ends_at IS NULL OR ends_at > now())
         RETURNING id",
        ids,
        by
    )
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(|r| r.id).collect())
}

/// 取消撤销（撤销了「解除处罚」），只恢复还没到期的；返回是否恢复。
pub async fn unrevoke(db: impl PgExecutor<'_>, id: i64) -> Result<bool, sqlx::Error> {
    let r = sqlx::query!(
        "UPDATE sanctions SET revoked_at = NULL, revoked_by = NULL
         WHERE id = $1 AND revoked_at IS NOT NULL AND (ends_at IS NULL OR ends_at > now())",
        id
    )
    .execute(db)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// 用户在 `forum_id` 板块（为空表示只看全站处罚）生效中的 `kinds` 处罚里编号最大的一种；
/// 发帖时查 `[禁言, 封号]`，封号优先。
pub async fn active_kind(
    db: impl PgExecutor<'_>,
    user_id: i64,
    kinds: &[i16],
    forum_id: Option<i32>,
) -> Result<Option<i16>, sqlx::Error> {
    let row = sqlx::query_file!(
        "queries/sanctions_active_kind.sql",
        user_id,
        kinds,
        forum_id
    )
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| r.kind))
}

/// 登录时：IP 落在被封网段内，或设备被封禁。
pub async fn access_banned(
    db: impl PgExecutor<'_>,
    ip: &str,
    device_hash: Option<&[u8]>,
) -> Result<bool, sqlx::Error> {
    let row = sqlx::query_file!("queries/sanctions_access.sql", ip, device_hash)
        .fetch_one(db)
        .await?;
    Ok(row.banned)
}
