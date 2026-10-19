//! 管理角色（plan 第 9 节）。后台登录只用 Passkey 在后台功能里实现；这里只判断权限。

use sqlx::PgExecutor;

pub const ROLE_OWNER: i16 = 1;
pub const ROLE_SUPER_ADMIN: i16 = 2;
pub const ROLE_ADMIN: i16 = 3;
pub const ROLE_MODERATOR: i16 = 4;
pub const ROLE_ARBITER: i16 = 5;

/// 能否管理该板块的主题：站长、超级管理员、管理员管全站，版主只管自己的板块。
pub async fn can_moderate(
    db: impl PgExecutor<'_>,
    user_id: i64,
    forum_id: i32,
) -> Result<bool, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT EXISTS (
             SELECT 1 FROM staff_roles
             WHERE user_id = $1 AND (role IN (1, 2, 3) OR (role = 4 AND forum_id = $2))
           ) AS "ok!""#,
        user_id,
        forum_id
    )
    .fetch_one(db)
    .await?;
    Ok(row.ok)
}

/// 授予角色（任免功能的最小实现）。版主必须指定板块，其余角色不能指定。
pub async fn grant(
    db: impl PgExecutor<'_>,
    user_id: i64,
    role: i16,
    forum_id: Option<i32>,
    granted_by: Option<i64>,
) -> Result<i64, sqlx::Error> {
    let row = sqlx::query!(
        "INSERT INTO staff_roles (user_id, role, forum_id, granted_by)
         VALUES ($1, $2, $3, $4) RETURNING id",
        user_id,
        role,
        forum_id,
        granted_by
    )
    .fetch_one(db)
    .await?;
    Ok(row.id)
}

/// 用户管理的范围：是否全站管理员，以及作为版主管辖的板块。
pub async fn scope(db: impl PgExecutor<'_>, user_id: i64) -> Result<(bool, Vec<i32>), sqlx::Error> {
    let rows = sqlx::query!(
        "SELECT role, forum_id FROM staff_roles WHERE user_id = $1 AND role IN (1, 2, 3, 4)",
        user_id
    )
    .fetch_all(db)
    .await?;
    let global = rows.iter().any(|r| r.role != ROLE_MODERATOR);
    Ok((
        global,
        rows.into_iter().filter_map(|r| r.forum_id).collect(),
    ))
}

/// 是否仲裁员（可查看全部订单详情，plan 第 7 节「仲裁规则」）。
pub async fn is_arbiter(db: impl PgExecutor<'_>, user_id: i64) -> Result<bool, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT EXISTS (SELECT 1 FROM staff_roles WHERE user_id = $1 AND role = $2) AS "ok!""#,
        user_id,
        ROLE_ARBITER
    )
    .fetch_one(db)
    .await?;
    Ok(row.ok)
}
