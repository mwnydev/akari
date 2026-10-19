//! 图片上传（plan 第 12 节）：申请预签名地址 → 浏览器直传临时目录 → 确认后排队 → worker 处理成 WebP。
//!
//! 队列就是本表（`FOR UPDATE SKIP LOCKED` 领取 + 租约），与发件箱一样是接入 apalis 之前的最小实现。

use serde_json::Value;
use sqlx::PgExecutor;
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "upload_status", rename_all = "lowercase")]
pub enum UploadStatus {
    /// 已发预签名地址，等浏览器上传完成后确认。
    Pending,
    Queued,
    Processing,
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Upload {
    pub id: i64,
    pub user_id: i64,
    pub status: UploadStatus,
    pub content_type: String,
    pub size: i32,
    pub tmp_key: String,
    pub width: Option<i32>,
    pub height: Option<i32>,
    /// `[{"width", "height", "url"}]`，从小到大。
    pub variants: Option<Value>,
    pub error: Option<String>,
    pub attempts: i32,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

pub async fn create(
    db: impl PgExecutor<'_>,
    user_id: i64,
    content_type: &str,
    size: i32,
    tmp_key: &str,
) -> Result<i64, sqlx::Error> {
    let row = sqlx::query_file!(
        "queries/upload_create.sql",
        user_id,
        content_type,
        size,
        tmp_key
    )
    .fetch_one(db)
    .await?;
    Ok(row.id)
}

pub async fn by_id(db: impl PgExecutor<'_>, id: i64) -> Result<Option<Upload>, sqlx::Error> {
    sqlx::query_file_as!(Upload, "queries/uploads_by_id.sql", id)
        .fetch_optional(db)
        .await
}

/// 上传者确认直传完成，排队处理；不是本人的、或已确认过的返回 `false`。
pub async fn queue(db: impl PgExecutor<'_>, id: i64, user_id: i64) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query_file!("queries/upload_queue.sql", id, user_id)
        .fetch_optional(db)
        .await?
        .is_some())
}

/// worker 领取的一张待处理图片。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claimed {
    pub id: i64,
    pub user_id: i64,
    pub tmp_key: String,
    pub size: i32,
    /// 含本次。
    pub attempts: i32,
}

/// 领取一张并续租 `lease_secs` 秒；没有待处理的返回 `None`。
pub async fn claim(
    db: impl PgExecutor<'_>,
    lease_secs: f64,
) -> Result<Option<Claimed>, sqlx::Error> {
    sqlx::query_file_as!(Claimed, "queries/upload_claim.sql", lease_secs)
        .fetch_optional(db)
        .await
}

pub async fn mark_done(
    db: impl PgExecutor<'_>,
    id: i64,
    width: i32,
    height: i32,
    variants: &Value,
) -> Result<(), sqlx::Error> {
    sqlx::query_file!("queries/upload_done.sql", id, width, height, variants)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn mark_failed(db: impl PgExecutor<'_>, id: i64, error: &str) -> Result<(), sqlx::Error> {
    sqlx::query_file!("queries/upload_failed.sql", id, error)
        .execute(db)
        .await?;
    Ok(())
}

/// 暂时失败：`delay_secs` 秒后由任一 worker 重新领取。
pub async fn retry_later(
    db: impl PgExecutor<'_>,
    id: i64,
    delay_secs: f64,
    error: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query_file!("queries/upload_retry_later.sql", id, delay_secs, error)
        .execute(db)
        .await?;
    Ok(())
}
