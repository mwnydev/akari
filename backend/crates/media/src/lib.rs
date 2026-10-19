//! 图片上传与处理（plan 第 12 节）：对象存储（R2 / S3 兼容）与 WebP 转换。

pub mod process;
pub mod storage;

use rand::RngExt;
use sha2::{Digest, Sha256};

pub use process::{Format, ImageProcessor, ProcessError, Processed, RustProcessor, Variant};
pub use storage::{PresignedPut, Storage, StorageConfig, StorageError};

/// 浏览器上传的最大字节数。
pub const MAX_UPLOAD_BYTES: u64 = 10 * 1024 * 1024;

/// 新的临时目录对象名（随机，不可猜）。
pub fn tmp_key(user_id: i64) -> String {
    let r: [u8; 16] = rand::rng().random();
    format!("{}{user_id}/{}", storage::TMP_PREFIX, hex::encode(r))
}

/// 正式目录对象名：内容的 SHA-256，同样的输出只存一份。
pub fn image_key(webp: &[u8]) -> String {
    format!(
        "{}{}.webp",
        storage::IMAGE_PREFIX,
        hex::encode(Sha256::digest(webp))
    )
}
