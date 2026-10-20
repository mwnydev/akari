//! 板块。板块表很小（几十行），列表直接全取。

use sqlx::PgExecutor;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Forum {
    pub id: i32,
    pub parent_id: Option<i32>,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub position: i32,
    /// 可见所需最低等级（游客为 0）。
    pub view_min_level: i16,
    pub post_min_level: i16,
    pub reply_min_level: i16,
    /// 能否发交易帖。
    pub allow_trade: bool,
    /// 能否发拍卖帖（拍卖区）。
    pub allow_auction: bool,
    /// 色块（固定色板里的 `#rrggbb`，迁移里有 CHECK）；为空时前端按位置轮换。
    pub color: Option<String>,
}

/// 板块色板（迁移 20261202000001 的 CHECK 与 frontend/apps/forum/app/globals.css 的 `--color-board-*`，改色板要几处一起改；
/// 测试核对它与数据库约束一致）。
pub const PALETTE: [&str; 6] = [
    "#d1453b", "#2f7fd8", "#2e9d5c", "#e08e1b", "#8059c9", "#1d9bb0",
];

/// 后台修改板块：为空的字段不改。
#[derive(Debug, Clone, Default)]
pub struct ForumPatch<'a> {
    pub name: Option<&'a str>,
    pub description: Option<&'a str>,
    /// `Some(None)` 清除色块（前端按位置轮换）。
    pub color: Option<Option<&'a str>>,
    pub position: Option<i32>,
    pub allow_trade: Option<bool>,
    pub allow_auction: Option<bool>,
}

/// 后台创建板块用的参数。
#[derive(Debug, Clone)]
pub struct NewForum<'a> {
    pub parent_id: Option<i32>,
    pub slug: &'a str,
    pub name: &'a str,
    pub description: &'a str,
    pub position: i32,
    pub view_min_level: i16,
    pub post_min_level: i16,
    pub reply_min_level: i16,
    pub allow_trade: bool,
    pub allow_auction: bool,
    pub color: Option<&'a str>,
}

pub async fn list_all(db: impl PgExecutor<'_>) -> Result<Vec<Forum>, sqlx::Error> {
    sqlx::query_as!(
        Forum,
        "SELECT id, parent_id, slug, name, description, position,
                view_min_level, post_min_level, reply_min_level, allow_trade, allow_auction, color
         FROM forums
         ORDER BY parent_id NULLS FIRST, position, id"
    )
    .fetch_all(db)
    .await
}

pub async fn by_slug(db: impl PgExecutor<'_>, slug: &str) -> Result<Option<Forum>, sqlx::Error> {
    sqlx::query_as!(
        Forum,
        "SELECT id, parent_id, slug, name, description, position,
                view_min_level, post_min_level, reply_min_level, allow_trade, allow_auction, color
         FROM forums WHERE slug = $1",
        slug
    )
    .fetch_optional(db)
    .await
}

pub async fn by_id(db: impl PgExecutor<'_>, id: i32) -> Result<Option<Forum>, sqlx::Error> {
    sqlx::query_as!(
        Forum,
        "SELECT id, parent_id, slug, name, description, position,
                view_min_level, post_min_level, reply_min_level, allow_trade, allow_auction, color
         FROM forums WHERE id = $1",
        id
    )
    .fetch_optional(db)
    .await
}

/// 创建板块（后台功能的最小实现；开发环境种子数据见 `seeds/dev.sql`）。
pub async fn create(db: impl PgExecutor<'_>, f: &NewForum<'_>) -> Result<i32, sqlx::Error> {
    let row = sqlx::query!(
        "INSERT INTO forums (parent_id, slug, name, description, position,
                             view_min_level, post_min_level, reply_min_level, allow_trade,
                             allow_auction, color)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) RETURNING id",
        f.parent_id,
        f.slug,
        f.name,
        f.description,
        f.position,
        f.view_min_level,
        f.post_min_level,
        f.reply_min_level,
        f.allow_trade,
        f.allow_auction,
        f.color
    )
    .fetch_one(db)
    .await?;
    Ok(row.id)
}

/// 修改板块（后台），返回修改后的板块；不存在返回 `None`。
pub async fn update(
    db: impl PgExecutor<'_>,
    id: i32,
    p: &ForumPatch<'_>,
) -> Result<Option<Forum>, sqlx::Error> {
    sqlx::query_as!(
        Forum,
        "UPDATE forums SET name = coalesce($2, name), description = coalesce($3, description),
                color = CASE WHEN $4 THEN $5 ELSE color END, position = coalesce($6, position),
                allow_trade = coalesce($7, allow_trade), allow_auction = coalesce($8, allow_auction)
         WHERE id = $1
         RETURNING id, parent_id, slug, name, description, position,
                   view_min_level, post_min_level, reply_min_level, allow_trade, allow_auction, color",
        id,
        p.name,
        p.description,
        p.color.is_some(),
        p.color.flatten(),
        p.position,
        p.allow_trade,
        p.allow_auction
    )
    .fetch_optional(db)
    .await
}
