//! `sig/SignatureCheck.hex` 必须是 `sig/SignatureCheck.sol` 的编译结果（单一事实来源）。
//! 没装 forge 或 jq 时跳过。不一致时运行 `backend/crates/chain/sig/regen.sh`。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

use std::path::Path;
use std::process::Command;

use forum_testkit::{require, tool};

#[test]
fn committed_bytecode_matches_the_source() {
    let (Some(forge), Some(_)) = (
        require(tool("forge"), "forge", "install Foundry"),
        require(tool("jq"), "jq", "install jq"),
    ) else {
        return;
    };
    let sig = Path::new(env!("CARGO_MANIFEST_DIR")).join("sig");
    let fresh = Path::new(env!("CARGO_TARGET_TMPDIR")).join("SignatureCheck.hex");
    let path = std::env::join_paths(forge.parent().map(Path::to_path_buf).into_iter().chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))
    .unwrap();
    let status = Command::new("bash")
        .arg(sig.join("regen.sh"))
        .arg(&fresh)
        .env("PATH", path)
        .status()
        .unwrap();
    assert!(status.success(), "regen.sh failed");
    let committed = std::fs::read_to_string(sig.join("SignatureCheck.hex")).unwrap();
    let fresh = std::fs::read_to_string(&fresh).unwrap();
    assert!(
        committed.trim() == fresh.trim(),
        "sig/SignatureCheck.hex is stale: run sig/regen.sh"
    );
}
