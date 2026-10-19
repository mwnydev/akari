#!/usr/bin/env bash
# 本地 AA 开发链：anvil fork Arbitrum Sepolia + 部署本仓库合约 + Pimlico Alto bundler。说明见 README-aa.md。
#   deploy/dev/aa.sh up      启动 anvil → 核对 AA 基础合约 → 部署 → 充押金 → 写 chain.local.json → 启动 Alto
#   deploy/dev/aa.sh down    停止并删除 anvil、Alto 容器（链状态不保留，下次 up 重新 fork）
#   deploy/dev/aa.sh status  显示两个服务是否在线
# 仅限开发：下面的私钥由固定字符串派生（cast keccak "akari-dev-only:<name>"），公开、无价值，绝不能用在任何真实网络。
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
CONTRACTS="$ROOT/contracts"
OUT="$ROOT/frontend/packages/contracts/src/chain.local.json"
COMPOSE=(docker compose -f "$HERE/compose.yaml" --profile aa)

export PATH="$HOME/.foundry/bin:$PATH"
export NO_PROXY="127.0.0.1,localhost" no_proxy="127.0.0.1,localhost"
RPC="http://127.0.0.1:${AA_ANVIL_PORT:-8545}"
BUNDLER="http://127.0.0.1:${AA_BUNDLER_PORT:-4337}"
CHAIN_ID=31337

DEPLOYER_KEY=0x2bd477e315997bfc5018a2d4ef4780999f011bceb8cf602c009337739f641d81 # akari-dev-only:deployer（本地 Anvil 专用，公开派生）gitleaks:allow
SPONSOR_ADDR=0x9A53c3ACF864004F00BdE6E48Bd8A50D6470e8A8                          # akari-dev-only:sponsor
EXECUTOR_ADDR=0x19d6319D75788D0fc7f71f3b49fa11863412c804                         # akari-dev-only:bundler-executor
UTILITY_ADDR=0x685bb2D07a60B63Bd337a0a67b02Ee7749a4Ade3                          # akari-dev-only:bundler-utility

# 已在 Arbitrum Sepolia 上用 eth_getCode 核对过（2026-10-19，README-aa.md 有表）；fork 后必须仍然有代码。
ENTRY_POINT=0x0000000071727De22E5E9d8BAf0edAc6f37da032
KERNEL_IMPL=0xd6CEDDe84be40893d153Be9d467CD6aD37875b28
KERNEL_FACTORY=0x2577507b78c2008Ff367261CB6285d44ba5eF2E9
KERNEL_META_FACTORY=0xd703aaE79538628d27099B8c4f621bE4CCd142d5
WEBAUTHN_VALIDATOR=0x7ab16Ff354AcB328452F1D445b3Ddee9a91e9e69 # gitleaks:allow 公开合约地址
ERC6492_VERIFIER=0x00007bd799e4A591FeA53f8A8a3E9f931626Ba7e

PAYMASTER_DEPOSIT=10ether
PAYMASTER_STAKE=1ether
PAYMASTER_UNSTAKE_DELAY=86400

log() { printf '\033[1;34m[aa]\033[0m %s\n' "$*" >&2; }
die() { printf '\033[1;31m[aa]\033[0m %s\n' "$*" >&2; exit 1; }

wait_for() { # wait_for <描述> <命令…>
  local what=$1; shift
  for _ in $(seq 1 90); do
    if "$@" >/dev/null 2>&1; then return 0; fi
    sleep 1
  done
  die "$what 没有在 90 秒内就绪"
}

bundler_up() {
  curl -sf -X POST -H 'content-type: application/json' \
    --data '{"jsonrpc":"2.0","id":1,"method":"eth_supportedEntryPoints","params":[]}' "$BUNDLER" |
    grep -qi "${ENTRY_POINT#0x}"
}

check_code() {
  local name=$1 addr=$2
  [[ "$(cast code "$addr" --rpc-url "$RPC")" != "0x" ]] || die "$name $addr 在 fork 上没有代码"
  log "  ✓ $name $addr"
}

up() {
  command -v forge >/dev/null || die "找不到 forge（~/.foundry/bin）"
  log "启动 anvil（fork ${AA_FORK_URL:-https://sepolia-rollup.arbitrum.io/rpc}）"
  "${COMPOSE[@]}" up -d --wait anvil
  [[ "$(cast chain-id --rpc-url "$RPC")" == "$CHAIN_ID" ]] || die "anvil 链 ID 不是 $CHAIN_ID"

  log "核对 AA 基础合约"
  check_code EntryPoint-v0.7 "$ENTRY_POINT"
  check_code Kernel-v3.3-implementation "$KERNEL_IMPL"
  check_code Kernel-v3.3-factory "$KERNEL_FACTORY"
  check_code Kernel-v3.3-meta-factory "$KERNEL_META_FACTORY"
  check_code WebAuthn-validator-0.0.3 "$WEBAUTHN_VALIDATOR"
  check_code ERC-6492-verifier "$ERC6492_VERIFIER"
  # P-256 预编译自检：RIP-7212 规范里的有效签名测试向量，返回 1 才算可用
  [[ "$(cast call 0x0000000000000000000000000000000000000100 --rpc-url "$RPC" \
    --data 0x4cee90eb86eaa050036147a12d49004b6b9c72bd725d39d4785011fe190f0b4da73bd4903f0ce3b639bbbf6e8e80d16931ff4bcf5993d58468e8fb19086e8cac36dbcd03009df8c59286b162af3bd7fcc0450c9aa81be5d10d312af6c66b1d604aebd3099c618202fcfe16ae7770b0c49ab5eadf74b754204a3bb6060e44eff37618b065f9832de4ca6ca971a7a1adc826d0f7c00181a5fb2ddf79ae00b4e10e)" == *1 ]] ||
    die "secp256r1 预编译（0x100）不可用，anvil 需要 --hardfork osaka"
  log "  ✓ secp256r1 预编译 0x100"

  local deployer
  deployer=$(cast wallet address "$DEPLOYER_KEY")
  for a in "$deployer" "$EXECUTOR_ADDR" "$UTILITY_ADDR"; do
    cast rpc anvil_setBalance "$a" 0x56bc75e2d63100000 --rpc-url "$RPC" >/dev/null # 100 ETH
  done
  [[ "$(cast nonce "$deployer" --rpc-url "$RPC")" == 0 ]] ||
    die "部署账户 nonce 不是 0（链没有重置？先 aa.sh down）"

  log "部署合约（部署账户 $deployer，nonce 从 0 开始，地址每次都相同）"
  (cd "$CONTRACTS" && forge build --quiet)
  local usdt config escrow paymaster
  usdt=$(cd "$CONTRACTS" && forge create src/MockUSDT.sol:MockUSDT --broadcast --json \
    --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" | jq -r .deployedTo)
  (cd "$CONTRACTS" && TOKEN=$usdt OWNER=$deployer ARBITER=$deployer FEE_RECIPIENT=$deployer \
    forge script script/Deploy.s.sol --broadcast --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" --silent)
  config=$(cast compute-address "$deployer" --nonce 1 | awk '{print $NF}')
  escrow=$(cast compute-address "$deployer" --nonce 2 | awk '{print $NF}')
  [[ "$(cast call "$escrow" 'config()(address)' --rpc-url "$RPC")" == "$config" ]] ||
    die "Escrow / EscrowConfig 地址与预期不符"
  paymaster=$(cd "$CONTRACTS" && forge create src/aa/FixedFeePaymaster.sol:FixedFeePaymaster --broadcast --json \
    --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" \
    --constructor-args "$ENTRY_POINT" "$usdt" "$deployer" "$SPONSOR_ADDR" "$deployer" | jq -r .deployedTo)

  log "Paymaster 押金 $PAYMASTER_DEPOSIT、质押 $PAYMASTER_STAKE"
  cast send "$paymaster" 'deposit()' --value "$PAYMASTER_DEPOSIT" \
    --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" >/dev/null
  cast send "$paymaster" 'addStake(uint32)' "$PAYMASTER_UNSTAKE_DELAY" --value "$PAYMASTER_STAKE" \
    --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" >/dev/null

  jq -n --argjson chainId "$CHAIN_ID" --arg rpc "$RPC" --arg bundler "$BUNDLER" \
    --arg ep "$ENTRY_POINT" --arg impl "$KERNEL_IMPL" --arg factory "$KERNEL_FACTORY" \
    --arg meta "$KERNEL_META_FACTORY" --arg webauthn "$WEBAUTHN_VALIDATOR" --arg v6492 "$ERC6492_VERIFIER" \
    --arg usdt "$usdt" --arg config "$config" --arg escrow "$escrow" --arg paymaster "$paymaster" \
    --arg signer "$SPONSOR_ADDR" --arg deployer "$deployer" '{
      "//": "由 deploy/dev/aa.sh 生成，勿手改。本地 anvil（fork Arbitrum Sepolia，链 ID 改为 31337）。",
      chainId: $chainId,
      name: "Akari 本地链（Arbitrum Sepolia fork）",
      rpcUrl: $rpc,
      bundlerUrl: $bundler,
      entryPoint: $ep,
      kernel: { version: "0.3.3", implementation: $impl, factory: $factory, metaFactory: $meta },
      webAuthnValidator: $webauthn,
      erc6492Verifier: $v6492,
      contracts: { MockUSDT: $usdt, EscrowConfig: $config, Escrow: $escrow, FixedFeePaymaster: $paymaster },
      paymasterSigner: $signer,
      deployer: $deployer
    }' >"$OUT"
  log "已写入 ${OUT#"$ROOT/"}"

  log "启动 Alto bundler"
  "${COMPOSE[@]}" up -d alto
  wait_for "Alto bundler（$BUNDLER）" bundler_up
  log "就绪：RPC $RPC · bundler $BUNDLER · fork 区块 $(cast rpc anvil_nodeInfo --rpc-url "$RPC" | jq -r .forkConfig.forkBlockNumber)"
}

down() {
  "${COMPOSE[@]}" rm -sf alto anvil
}

status() {
  if cast chain-id --rpc-url "$RPC" >/dev/null 2>&1; then log "anvil   在线 $RPC"; else log "anvil   未运行"; fi
  if bundler_up; then log "bundler 在线 $BUNDLER"; else log "bundler 未运行"; fi
}

case "${1:-}" in
  up) up ;;
  down) down ;;
  status) status ;;
  *) die "用法：$0 up|down|status" ;;
esac
