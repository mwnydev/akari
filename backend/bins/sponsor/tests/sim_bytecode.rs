//! `sim/SponsorSimulations.hex` 必须是 `sim/SponsorSimulations.sol` 的编译结果（单一事实来源）。
//! 没装 forge 或 jq 时跳过。不一致时运行 `backend/bins/sponsor/sim/regen.sh`。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn tool(name: &str) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".foundry/bin").join(name));
    home.filter(|p| p.is_file()).or_else(|| {
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

#[test]
fn committed_bytecode_matches_the_source() {
    let (Some(forge), Some(_)) = (tool("forge"), tool("jq")) else {
        eprintln!("skipped: forge or jq not installed");
        return;
    };
    let sim = Path::new(env!("CARGO_MANIFEST_DIR")).join("sim");
    let fresh = Path::new(env!("CARGO_TARGET_TMPDIR")).join("SponsorSimulations.hex");
    let path = std::env::join_paths(forge.parent().map(Path::to_path_buf).into_iter().chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))
    .unwrap();
    let status = Command::new("bash")
        .arg(sim.join("regen.sh"))
        .arg(&fresh)
        .env("PATH", path)
        .status()
        .unwrap();
    assert!(status.success(), "regen.sh failed");
    let committed = std::fs::read_to_string(sim.join("SponsorSimulations.hex")).unwrap();
    let fresh = std::fs::read_to_string(&fresh).unwrap();
    assert!(
        committed.trim() == fresh.trim(),
        "sim/SponsorSimulations.hex is stale: run sim/regen.sh"
    );
}
