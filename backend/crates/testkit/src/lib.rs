//! 测试的外部前提（anvil、forge、jq、Valkey、S3 兼容存储、合约产物）。缺了默认打印 `skipped: …` 并跳过；
//! `AKARI_TEST_STRICT=1`（CI 设置）时直接失败，免得前提没装好时测试悄悄「通过」。

use std::path::PathBuf;
use std::process::{Command, Stdio};

/// 缺少测试前提 `what`（`reason` 说明怎么补上）：严格模式下 panic，否则打印 `skipped: …`，由调用方返回。
#[track_caller]
pub fn require_or_skip(what: &str, reason: &str) {
    if std::env::var("AKARI_TEST_STRICT").is_ok_and(|v| v == "1") {
        panic!(
            "test prerequisite missing: {what} ({reason}); AKARI_TEST_STRICT=1 forbids skipping"
        );
    }
    eprintln!("skipped: {what} not available ({reason})");
}

/// `value` 为空时按 [`require_or_skip`] 处理；原样返回。
#[track_caller]
pub fn require<T>(value: Option<T>, what: &str, reason: &str) -> Option<T> {
    if value.is_none() {
        require_or_skip(what, reason);
    }
    value
}

/// 非空的 `VALKEY_URL`，缺了按 [`require_or_skip`] 处理。
#[track_caller]
pub fn valkey_url() -> Option<String> {
    let url = std::env::var("VALKEY_URL").ok().filter(|v| !v.is_empty());
    require(url, "VALKEY_URL", "see deploy/dev/compose.yaml `valkey`")
}

/// S3 兼容存储的配置（`MEDIA_S3_*`，由 `forum_media::StorageConfig::from_env` 读出）。
#[track_caller]
pub fn require_s3<T>(config: Option<T>) -> Option<T> {
    require(
        config,
        "MEDIA_S3_*",
        "see deploy/dev/compose.yaml `s3` and backend/.env.example",
    )
}

/// 找命令行工具：`~/.foundry/bin/<name>` 或 PATH。找不到返回 `None`（调用方再交给 [`require`]）。
pub fn tool(name: &str) -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join(".foundry/bin").join(name))
        .filter(|p| p.is_file())
        .or_else(|| {
            Command::new(name)
                .arg("--version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .ok()
                .filter(|s| s.success())
                .map(|_| PathBuf::from(name))
        })
}
