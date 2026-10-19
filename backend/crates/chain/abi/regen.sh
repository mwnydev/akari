#!/usr/bin/env bash
# 从 Foundry 产物重新生成 Rust 绑定所用的 ABI 副本（plan 第 14 节：合约接口唯一定义处是 Solidity 源码）。
# 只保留 abi 与创建字节码（集成测试用它在 anvil 上部署），后端编译不需要安装 Foundry。
#
#   backend/crates/chain/abi/regen.sh        # 需要 forge 与 jq
#
# 合约改动后运行本脚本并提交 abi/*.json；`cargo test -p forum-chain` 会在 contracts/out 存在时校验副本没有过期。
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
contracts="$here/../../../../contracts"
(cd "$contracts" && forge build --quiet)
for c in Escrow EscrowConfig MockUSDT; do
  jq '{abi: .abi, bytecode: {object: .bytecode.object}}' "$contracts/out/$c.sol/$c.json" > "$here/$c.json"
done
# alloy 的 sol! 元组最多 24 个字段，Escrow.Order 有 27 个：getOrder 单独存一份片段，由 dyn-abi 解码（src/snapshot.rs）。
jq '[.abi[] | select(.type == "function" and .name == "getOrder")][0]' "$here/Escrow.json" > "$here/Escrow.getOrder.json"
jq '.abi |= map(select(.name != "getOrder"))' "$here/Escrow.json" > "$here/Escrow.json.tmp"
mv "$here/Escrow.json.tmp" "$here/Escrow.json"
echo "regenerated: $(ls "$here"/*.json | xargs -n1 basename | tr '\n' ' ')"
