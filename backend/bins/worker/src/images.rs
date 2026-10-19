//! 图片转换（plan 第 12 节）：领取排队的上传 → 取回原图 → 按内容判断格式、清除 EXIF、转 WebP 三种尺寸
//! （`spawn_blocking`，同时最多 [`CONCURRENCY`] 张）→ 内容哈希命名存正式目录 → 删除原图。

use std::sync::Arc;
use std::time::Duration;

use forum_db::{PgPool, uploads};
use forum_media::{ImageProcessor, MAX_UPLOAD_BYTES, Storage, StorageError};
use serde_json::json;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

/// 同时转换的图片数（plan 第 12 节「转换限 2 个 CPU 核」）。
pub const CONCURRENCY: usize = 2;
/// 领取后的租约：超过这个时间还没处理完（worker 崩溃）就由别的 worker 重新领取。
const LEASE_SECS: f64 = 120.0;
/// 存储暂时失败时的最多尝试次数，之后标为失败。
const MAX_ATTEMPTS: i32 = 5;
const IDLE_POLL: Duration = Duration::from_secs(1);

#[derive(Clone)]
pub struct ImageWorker {
    pub db: PgPool,
    pub storage: Arc<Storage>,
    pub processor: Arc<dyn ImageProcessor>,
}

/// 一张图片的处理结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Done,
    Failed(&'static str),
    /// 存储暂时不可用，稍后重试。
    Retry,
}

impl ImageWorker {
    /// 领取并处理一张；没有待处理的返回 `None`。
    pub async fn run_once(&self) -> Result<Option<(i64, Outcome)>, sqlx::Error> {
        let Some(job) = uploads::claim(&self.db, LEASE_SECS).await? else {
            return Ok(None);
        };
        let outcome = self.handle(&job).await?;
        Ok(Some((job.id, outcome)))
    }

    async fn handle(&self, job: &uploads::Claimed) -> Result<Outcome, sqlx::Error> {
        let max = usize::try_from(MAX_UPLOAD_BYTES).unwrap_or(usize::MAX);
        let original = match self.storage.get(&job.tmp_key, max).await {
            Ok(Some(b)) => b,
            Ok(None) => return self.fail(job, "missing").await,
            Err(StorageError::TooLarge(_)) => return self.fail(job, "too_large").await,
            Err(e) => return self.retry(job, &e).await,
        };
        let processor = Arc::clone(&self.processor);
        let processed =
            match tokio::task::spawn_blocking(move || processor.process(&original)).await {
                Ok(Ok(p)) => p,
                Ok(Err(e)) => return self.fail(job, e.code()).await,
                Err(e) => {
                    tracing::error!(id = job.id, error = %e, "image processing panicked");
                    return self.fail(job, "corrupt").await;
                }
            };
        let mut variants = Vec::with_capacity(processed.variants.len());
        for v in processed.variants {
            let key = forum_media::image_key(&v.webp);
            let url = self.storage.public_url(&key);
            if let Err(e) = self.storage.put_public(&key, "image/webp", v.webp).await {
                return self.retry(job, &e).await;
            }
            variants.push(json!({ "width": v.width, "height": v.height, "url": url }));
        }
        let dim = |v: u32| i32::try_from(v).unwrap_or(i32::MAX);
        uploads::mark_done(
            &self.db,
            job.id,
            dim(processed.width),
            dim(processed.height),
            &json!(variants),
        )
        .await?;
        self.delete_original(job).await;
        Ok(Outcome::Done)
    }

    async fn fail(
        &self,
        job: &uploads::Claimed,
        code: &'static str,
    ) -> Result<Outcome, sqlx::Error> {
        uploads::mark_failed(&self.db, job.id, code).await?;
        self.delete_original(job).await;
        Ok(Outcome::Failed(code))
    }

    async fn retry(
        &self,
        job: &uploads::Claimed,
        e: &StorageError,
    ) -> Result<Outcome, sqlx::Error> {
        tracing::warn!(id = job.id, attempts = job.attempts, error = %e, "image storage failed");
        if job.attempts >= MAX_ATTEMPTS {
            return self.fail(job, "storage").await;
        }
        uploads::retry_later(&self.db, job.id, 30.0 * f64::from(job.attempts), "storage").await?;
        Ok(Outcome::Retry)
    }

    /// 删除临时目录里的原图；失败只记日志（存储桶生命周期规则会兜底清理临时目录）。
    async fn delete_original(&self, job: &uploads::Claimed) {
        if let Err(e) = self.storage.delete(&job.tmp_key).await {
            tracing::warn!(id = job.id, error = %e, "deleting original upload failed");
        }
    }

    /// 持续处理到 `shutdown`：最多 [`CONCURRENCY`] 张同时进行，没有待处理的就隔 1 秒再查。
    pub async fn run(self, shutdown: CancellationToken) {
        let permits = Arc::new(Semaphore::new(CONCURRENCY));
        loop {
            let permit = tokio::select! {
                () = shutdown.cancelled() => break,
                p = Arc::clone(&permits).acquire_owned() => match p {
                    Ok(p) => p,
                    Err(_) => break,
                },
            };
            let claimed = match uploads::claim(&self.db, LEASE_SECS).await {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!(error = %e, "claiming image upload failed");
                    None
                }
            };
            let Some(job) = claimed else {
                drop(permit);
                tokio::select! {
                    () = shutdown.cancelled() => break,
                    () = tokio::time::sleep(IDLE_POLL) => continue,
                }
            };
            let worker = self.clone();
            tokio::spawn(async move {
                match worker.handle(&job).await {
                    Ok(Outcome::Failed(code)) => {
                        tracing::info!(id = job.id, code, "image rejected")
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::error!(id = job.id, error = %e, "image job bookkeeping failed")
                    }
                }
                drop(permit);
            });
        }
        // 等进行中的图片处理完
        let _ = permits
            .acquire_many(u32::try_from(CONCURRENCY).unwrap_or(u32::MAX))
            .await;
    }
}
