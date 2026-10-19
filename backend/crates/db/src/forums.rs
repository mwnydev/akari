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
}

pub async fn list_all(db: impl PgExecutor<'_>) -> Result<Vec<Forum>, sqlx::Error> {
    sqlx::query_as!(
        Forum,
        "SELECT id, parent_id, slug, name, description, position,
                view_min_level, post_min_level, reply_min_level
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
                view_min_level, post_min_level, reply_min_level
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
                view_min_level, post_min_level, reply_min_level
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
                             view_min_level, post_min_level, reply_min_level)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id",
        f.parent_id,
        f.slug,
        f.name,
        f.description,
        f.position,
        f.view_min_level,
        f.post_min_level,
        f.reply_min_level
    )
    .fetch_one(db)
    .await?;
    Ok(row.id)
}
