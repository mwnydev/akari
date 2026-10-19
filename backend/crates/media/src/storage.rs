//! 对象存储（生产 Cloudflare R2，本地 S3 兼容服务）：rusty-s3 只做 SigV4 预签名，请求由 reqwest 发出。
//!
//! - 浏览器上传：预签名 `PUT` 到临时目录，`Content-Type` 与 `Content-Length` 都在签名里，
//!   浏览器只能按申请时声明的类型和字节数上传（R2 不支持带 content-length-range 的 POST 表单上传）。
//! - 正式目录的对象名是内容哈希，带一年的 `Cache-Control`，经 CDN 公开读取。
//! - 临时目录另由存储桶生命周期规则兜底清理（1 天），worker 处理完会立即删除原图。

use std::time::Duration;

use rusty_s3::actions::S3Action;
use rusty_s3::{Bucket, Credentials, UrlStyle};

/// 临时目录前缀（浏览器直传的原图）。
pub const TMP_PREFIX: &str = "tmp/";
/// 正式目录前缀（处理后的 WebP）。
pub const IMAGE_PREFIX: &str = "i/";
/// 浏览器上传地址有效期。
pub const UPLOAD_URL_TTL: Duration = Duration::from_secs(600);
/// 服务端自己发的请求的签名有效期。
const SERVER_URL_TTL: Duration = Duration::from_secs(60);
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("storage config: {0}")]
    Config(String),
    #[error("storage request: {0}")]
    Http(#[from] reqwest::Error),
    #[error("storage returned {0}")]
    Status(reqwest::StatusCode),
    #[error("object larger than {0} bytes")]
    TooLarge(usize),
}

#[derive(Debug, Clone)]
pub struct StorageConfig {
    /// S3 端点，如 `https://<account>.r2.cloudflarestorage.com` 或 `http://127.0.0.1:7070`。
    pub endpoint: String,
    pub bucket: String,
    /// R2 为 `auto`。
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
    /// 正式目录的公开地址前缀（CDN 或存储桶的公开域名），如 `https://img.example.com/`。
    pub public_base: String,
}

impl StorageConfig {
    /// 从环境变量读取：`MEDIA_S3_ENDPOINT`、`MEDIA_S3_BUCKET`、`MEDIA_S3_REGION`（默认 `auto`）、
    /// `MEDIA_S3_ACCESS_KEY`、`MEDIA_S3_SECRET_KEY`、`MEDIA_PUBLIC_BASE`。都没设置返回 `None`（图片上传不可用），
    /// 只设置了一部分是配置错误。
    pub fn from_env() -> Result<Option<Self>, StorageError> {
        let get = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let names = [
            "MEDIA_S3_ENDPOINT",
            "MEDIA_S3_BUCKET",
            "MEDIA_S3_ACCESS_KEY",
            "MEDIA_S3_SECRET_KEY",
            "MEDIA_PUBLIC_BASE",
        ];
        let values: Vec<Option<String>> = names.iter().map(|k| get(k)).collect();
        if values.iter().all(Option::is_none) {
            return Ok(None);
        }
        let mut it = values.into_iter().zip(names);
        let mut next = || match it.next() {
            Some((Some(v), _)) => Ok(v),
            Some((None, k)) => Err(StorageError::Config(format!("missing env {k}"))),
            None => Err(StorageError::Config("missing env".into())),
        };
        Ok(Some(Self {
            endpoint: next()?,
            bucket: next()?,
            access_key: next()?,
            secret_key: next()?,
            public_base: next()?,
            region: get("MEDIA_S3_REGION").unwrap_or_else(|| "auto".to_owned()),
        }))
    }
}

/// 浏览器直传用的预签名地址：请求必须带上 `headers` 里的每个头，值完全一致。
#[derive(Debug, Clone)]
pub struct PresignedPut {
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
}

#[derive(Clone)]
pub struct Storage {
    bucket: Bucket,
    credentials: Credentials,
    http: reqwest::Client,
    public_base: reqwest::Url,
}

impl std::fmt::Debug for Storage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Storage")
            .field("bucket", &self.bucket.name())
            .field("public_base", &self.public_base.as_str())
            .finish_non_exhaustive()
    }
}

impl Storage {
    pub fn new(cfg: &StorageConfig) -> Result<Self, StorageError> {
        let bad = |e: &dyn std::fmt::Display| StorageError::Config(e.to_string());
        let endpoint: reqwest::Url = cfg.endpoint.parse().map_err(|e| bad(&e))?;
        let bucket = Bucket::new(
            endpoint,
            UrlStyle::Path,
            cfg.bucket.clone(),
            cfg.region.clone(),
        )
        .map_err(|e| bad(&e))?;
        let mut public_base: reqwest::Url = cfg.public_base.parse().map_err(|e| bad(&e))?;
        if !public_base.path().ends_with('/') {
            let path = format!("{}/", public_base.path());
            public_base.set_path(&path);
        }
        if public_base.host_str().is_none() {
            return Err(StorageError::Config("public base needs a host".into()));
        }
        Ok(Self {
            bucket,
            credentials: Credentials::new(cfg.access_key.clone(), cfg.secret_key.clone()),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()?,
            public_base,
        })
    }

    /// 正式图片的公开主机名（帖子 HTML 里只允许这个主机的图片）。
    pub fn public_host(&self) -> &str {
        self.public_base.host_str().unwrap_or_default()
    }

    pub fn public_url(&self, key: &str) -> String {
        format!("{}{key}", self.public_base)
    }

    /// 浏览器直传临时目录的预签名 `PUT`。
    pub fn presign_upload(&self, key: &str, content_type: &str, size: u64) -> PresignedPut {
        let size = size.to_string();
        let mut action = self.bucket.put_object(Some(&self.credentials), key);
        action.headers_mut().insert("content-type", content_type);
        action.headers_mut().insert("content-length", size.clone());
        PresignedPut {
            url: action.sign(UPLOAD_URL_TTL).to_string(),
            headers: vec![
                ("content-type", content_type.to_owned()),
                ("content-length", size),
            ],
        }
    }

    /// 取对象，超过 `max_len` 字节即放弃；不存在返回 `None`。
    pub async fn get(&self, key: &str, max_len: usize) -> Result<Option<Vec<u8>>, StorageError> {
        let url = self
            .bucket
            .get_object(Some(&self.credentials), key)
            .sign(SERVER_URL_TTL);
        let mut res = self.http.get(url).send().await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !res.status().is_success() {
            return Err(StorageError::Status(res.status()));
        }
        let mut body = Vec::new();
        while let Some(chunk) = res.chunk().await? {
            if body.len() + chunk.len() > max_len {
                return Err(StorageError::TooLarge(max_len));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(Some(body))
    }

    /// 写入正式目录（内容哈希命名，CDN 缓存一年）。
    pub async fn put_public(
        &self,
        key: &str,
        content_type: &str,
        body: Vec<u8>,
    ) -> Result<(), StorageError> {
        let mut action = self.bucket.put_object(Some(&self.credentials), key);
        action.headers_mut().insert("content-type", content_type);
        action.headers_mut().insert("cache-control", IMMUTABLE);
        let url = action.sign(SERVER_URL_TTL);
        let res = self
            .http
            .put(url)
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .header(reqwest::header::CACHE_CONTROL, IMMUTABLE)
            .body(body)
            .send()
            .await?;
        ok(res.status())
    }

    /// 删除对象（不存在也算成功）。
    pub async fn delete(&self, key: &str) -> Result<(), StorageError> {
        let url = self
            .bucket
            .delete_object(Some(&self.credentials), key)
            .sign(SERVER_URL_TTL);
        let res = self.http.delete(url).send().await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        ok(res.status())
    }

    /// 建存储桶（本地开发与测试用；已存在也算成功）。
    pub async fn ensure_bucket(&self) -> Result<(), StorageError> {
        let url = self
            .bucket
            .create_bucket(&self.credentials)
            .sign(SERVER_URL_TTL);
        let res = self.http.put(url).send().await?;
        if res.status() == reqwest::StatusCode::CONFLICT {
            return Ok(());
        }
        ok(res.status())
    }
}

fn ok(status: reqwest::StatusCode) -> Result<(), StorageError> {
    if status.is_success() {
        Ok(())
    } else {
        Err(StorageError::Status(status))
    }
}
