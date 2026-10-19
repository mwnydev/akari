#!/usr/bin/env bash
# 重新编译 forum-sponsor 备用模拟用的 SponsorSimulations，只保留运行时字节码（eth_call 状态覆盖用，不部署）。
# 后端编译不需要 Foundry；改了 .sol 或升级 contracts/lib/account-abstraction 后运行本脚本并提交 .hex。
# `cargo test -p forum-sponsor --test sim_bytecode` 在装了 forge 时校验副本没有过期（编译到临时文件再比较）。
#
#   backend/bins/sponsor/sim/regen.sh [输出文件]     # 需要 forge 与 jq；默认写 sim/SponsorSimulations.hex
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
out="${1:-$here/SponsorSimulations.hex}"
lib="$(cd "$here/../../../../contracts/lib" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir "$work/src"
cp "$here/SponsorSimulations.sol" "$work/src/"
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
remappings = ["account-abstraction/=$lib/account-abstraction/contracts/", "@openzeppelin/contracts/=$lib/openzeppelin-contracts/contracts/"]
TOML
forge build --quiet --root "$work"
jq -r '.deployedBytecode.object' "$work/out/SponsorSimulations.sol/SponsorSimulations.json" >"$out"
echo "regenerated: $out"
