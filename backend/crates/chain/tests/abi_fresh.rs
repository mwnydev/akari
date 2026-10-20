//! `abi/*.json` 必须与 Foundry 产物一致（单一事实来源）。没有 `contracts/out`（未装 Foundry）时跳过。
//! 不一致时运行 `backend/crates/chain/abi/regen.sh`。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

use std::path::Path;

use serde_json::Value;

fn read(p: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

#[test]
fn committed_abi_matches_forge_artifacts() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = root.join("../../../contracts/out");
    if !out.exists() {
        forum_testkit::require_or_skip("contracts/out", "cd contracts && forge build");
        return;
    }
    for name in [
        "Escrow",
        "EscrowConfig",
        "StreamEscrow",
        "FixedFeePaymaster",
        "MockUSDT",
    ] {
        let forge = read(&out.join(format!("{name}.sol/{name}.json")));
        let Value::Array(mut abi) = forge["abi"].clone() else {
            panic!("{name}: no abi array");
        };
        // getOrder 的返回结构超过 sol! 元组上限，单独存放（见 regen.sh）。
        if matches!(name, "Escrow" | "StreamEscrow") {
            let get_order = abi
                .iter()
                .find(|f| f["name"] == "getOrder")
                .cloned()
                .unwrap();
            assert_eq!(
                get_order,
                read(&root.join(format!("abi/{name}.getOrder.json"))),
                "{name}.getOrder"
            );
            abi.retain(|f| f["name"] != "getOrder");
        }
        let committed = read(&root.join(format!("abi/{name}.json")));
        assert_eq!(
            Value::Array(abi),
            committed["abi"],
            "{name} abi stale: run abi/regen.sh"
        );
    }
}
