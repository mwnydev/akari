#!/usr/bin/env bash
# 重新编译 forum-api 验签用的 SignatureCheck，只保留创建字节码（eth_call 不带 `to` 执行构造函数，不部署）。
# 后端编译不需要 Foundry；改了 .sol 或升级 contracts/lib/solady 后运行本脚本并提交 .hex。
# `cargo test -p forum-chain --test sig_bytecode` 在装了 forge 时校验副本没有过期（编译到临时文件再比较）。
#
#   backend/crates/chain/sig/regen.sh [输出文件]     # 需要 forge 与 jq；默认写 sig/SignatureCheck.hex
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
out="${1:-$here/SignatureCheck.hex}"
lib="$(cd "$here/../../../../contracts/lib" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir "$work/src"
cp "$here/SignatureCheck.sol" "$work/src/"
cat >"$work/foundry.toml" <<TOML
[profile.default]
src = "src"
out = "out"
libs = ["$lib"]
allow_paths = ["$lib"]
solc_version = "0.8.37"
evm_version = "cancun"
optimizer = true
optimizer_runs = 200
# 不带元数据哈希：字节码只取决于源码与编译设置，校验测试可以逐字节比较。
cbor_metadata = false
bytecode_hash = "none"
remappings = ["solady/=$lib/solady/src/"]
TOML
forge build --quiet --root "$work"
jq -r '.bytecode.object' "$work/out/SignatureCheck.sol/SignatureCheck.json" >"$out"
echo "regenerated: $out"
