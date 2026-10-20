#!/usr/bin/env bash
# 本地 AA 开发链：anvil fork Arbitrum Sepolia + 部署本仓库合约 + Pimlico Alto bundler。说明见 README-aa.md。
#   deploy/dev/aa.sh up      启动 anvil → 核对 AA 基础合约 → 部署 → 充押金 → 核对前端链配置 → 写后端链配置与 .env.aa → 启动 Alto
#   deploy/dev/aa.sh down    停止并删除 anvil、Alto 容器与生成的后端链配置（链状态不保留，下次 up 重新 fork）
#   deploy/dev/aa.sh status  显示两个服务是否在线
# 端口：ANVIL_PORT（默认 8545）、BUNDLER_PORT（默认 4337）；同一台机器上多份检出同时运行时再设 AA_PROJECT（compose 项目名）。
# 仅限开发：下面的私钥由固定字符串派生（cast keccak "akari-dev-only:<name>"），公开、无价值，绝不能用在任何真实网络。
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
CONTRACTS="$ROOT/contracts"
# 前端链配置入库，内容固定（地址由部署账户 nonce 决定，URL 用默认端口）：up 只核对、不改写，见 README-aa.md。
CHAIN_JSON="$ROOT/frontend/packages/contracts/src/chain.local.json"
# 后端链配置（forum-api / forum-indexer / forum-sponsor 的 CHAIN_CONFIG）与开发环境变量；含部署区块，每次 up 都不同，不入库。
CHAIN_TOML="$HERE/chain.local.toml"
DEV_ENV="$HERE/.env.aa"
DEFAULT_ANVIL_PORT=8545
DEFAULT_BUNDLER_PORT=4337
ANVIL_PORT=${ANVIL_PORT:-$DEFAULT_ANVIL_PORT}
BUNDLER_PORT=${BUNDLER_PORT:-$DEFAULT_BUNDLER_PORT}
export ANVIL_PORT BUNDLER_PORT # compose.yaml 用
COMPOSE=(docker compose -p "${AA_PROJECT:-akari-dev}" -f "$HERE/compose.yaml" --profile aa)

export PATH="$HOME/.foundry/bin:$PATH"
export NO_PROXY="127.0.0.1,localhost" no_proxy="127.0.0.1,localhost"
RPC="http://127.0.0.1:$ANVIL_PORT"
BUNDLER="http://127.0.0.1:$BUNDLER_PORT"
CHAIN_ID=31337

DEPLOYER_KEY=0x2bd477e315997bfc5018a2d4ef4780999f011bceb8cf602c009337739f641d81 # akari-dev-only:deployer（本地 Anvil 专用，公开派生）gitleaks:allow
SPONSOR_KEY=0xdccc2a137d5ed46c4b46ea12b6e50feb5565cfa60bda1c3814b0e25d3f52eb98    # akari-dev-only:sponsor gitleaks:allow
SPONSOR_ADDR=0x9A53c3ACF864004F00BdE6E48Bd8A50D6470e8A8                          # akari-dev-only:sponsor
RELAYER_KEY=0x418cc49efb0f686af9f810b10928711756343553a8bcf6e77f784ec4c23310cf    # akari-dev-only:relayer gitleaks:allow
RELAYER_ADDR=0x8204490c73DceDE91340e855279A8A67758609AC                          # akari-dev-only:relayer（forum-sponsor 代发热钱包）
AUCTION_KEY=0x0484655bc83a9203012cc81cf5e00787e18dfa491046ed71f6ebe3d6fe11d5e9    # akari-dev-only:auction-signer gitleaks:allow
AUCTION_ADDR=0x66A9EF9f3699A02056f0abd5D26Eb7EA4b3ce9B3                          # akari-dev-only:auction-signer（EscrowConfig.auctionSigner）
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
# forum-api → forum-sponsor（只听本机）的地址与共享令牌，仅开发。
SPONSOR_PORT=${AA_SPONSOR_PORT:-8090}
SPONSOR_API_TOKEN=dev-only-sponsor-token-change-me-0123456789

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
  for a in "$deployer" "$EXECUTOR_ADDR" "$UTILITY_ADDR" "$RELAYER_ADDR"; do
    cast rpc anvil_setBalance "$a" 0x56bc75e2d63100000 --rpc-url "$RPC" >/dev/null # 100 ETH
  done
  [[ "$(cast nonce "$deployer" --rpc-url "$RPC")" == 0 ]] ||
    die "部署账户 nonce 不是 0（链没有重置？先 aa.sh down）"

  log "部署合约（部署账户 $deployer，nonce 从 0 开始，地址每次都相同）"
  (cd "$CONTRACTS" && forge build --quiet)
  local start_block usdt config escrow stream paymaster
  start_block=$(cast block-number --rpc-url "$RPC")
  usdt=$(cd "$CONTRACTS" && forge create src/MockUSDT.sol:MockUSDT --broadcast --json \
    --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" | jq -r .deployedTo)
  (cd "$CONTRACTS" && TOKEN=$usdt OWNER=$deployer ARBITER=$deployer FEE_RECIPIENT=$deployer AUCTION_SIGNER=$AUCTION_ADDR \
    forge script script/Deploy.s.sol --broadcast --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" --silent)
  config=$(cast compute-address "$deployer" --nonce 1 | awk '{print $NF}')
  escrow=$(cast compute-address "$deployer" --nonce 2 | awk '{print $NF}')
  stream=$(cast compute-address "$deployer" --nonce 3 | awk '{print $NF}')
  [[ "$(cast call "$escrow" 'config()(address)' --rpc-url "$RPC")" == "$config" ]] ||
    die "Escrow / EscrowConfig 地址与预期不符"
  [[ "$(cast call "$stream" 'escrow()(address)' --rpc-url "$RPC")" == "$escrow" ]] ||
    die "StreamEscrow 地址与预期不符"
  paymaster=$(cd "$CONTRACTS" && forge create src/aa/FixedFeePaymaster.sol:FixedFeePaymaster --broadcast --json \
    --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" \
    --constructor-args "$ENTRY_POINT" "$usdt" "$deployer" "$SPONSOR_ADDR" "$deployer" | jq -r .deployedTo)

  log "Paymaster 押金 $PAYMASTER_DEPOSIT、质押 $PAYMASTER_STAKE"
  cast send "$paymaster" 'deposit()' --value "$PAYMASTER_DEPOSIT" \
    --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" >/dev/null
  cast send "$paymaster" 'addStake(uint32)' "$PAYMASTER_UNSTAKE_DELAY" --value "$PAYMASTER_STAKE" \
    --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" >/dev/null

  check_chain_json "$usdt" "$config" "$escrow" "$stream" "$paymaster" "$deployer"
  write_backend_config "$start_block" "$usdt" "$config" "$escrow" "$stream" "$paymaster"

  log "启动 Alto bundler"
  "${COMPOSE[@]}" up -d alto
  wait_for "Alto bundler（$BUNDLER）" bundler_up
  check_loopback "$BUNDLER_PORT" "Alto bundler"
  check_loopback "$ANVIL_PORT" anvil
  log "就绪：RPC $RPC · bundler $BUNDLER · fork 区块 $(cast rpc anvil_nodeInfo --rpc-url "$RPC" | jq -r .forkConfig.forkBlockNumber)"
}

# 前端链配置（入库）：只含固定内容——部署地址（部署账户从 nonce 0 开始，每次相同）与默认端口的 URL。
# 与本次部署不符时停下（不悄悄改写受版本控制的文件）；确认是有意的改动（如部署脚本多了一笔交易）后用
# AA_WRITE_CHAIN_JSON=1 重新 up 改写它并提交。
check_chain_json() {
  local usdt=$1 config=$2 escrow=$3 stream=$4 paymaster=$5 deployer=$6 expected
  expected=$(jq -n --argjson chainId "$CHAIN_ID" \
    --arg rpc "http://127.0.0.1:$DEFAULT_ANVIL_PORT" --arg bundler "http://127.0.0.1:$DEFAULT_BUNDLER_PORT" \
    --arg ep "$ENTRY_POINT" --arg impl "$KERNEL_IMPL" --arg factory "$KERNEL_FACTORY" \
    --arg meta "$KERNEL_META_FACTORY" --arg webauthn "$WEBAUTHN_VALIDATOR" --arg v6492 "$ERC6492_VERIFIER" \
    --arg usdt "$usdt" --arg config "$config" --arg escrow "$escrow" --arg stream "$stream" \
    --arg paymaster "$paymaster" \
    --arg signer "$SPONSOR_ADDR" --arg deployer "$deployer" '{
      "//": "本地 anvil（fork Arbitrum Sepolia，链 ID 改为 31337）的固定配置，deploy/dev/aa.sh up 核对；改动方法见 deploy/dev/README-aa.md。",
      chainId: $chainId,
      name: "Akari 本地链（Arbitrum Sepolia fork）",
      rpcUrl: $rpc,
      bundlerUrl: $bundler,
      entryPoint: $ep,
      kernel: { version: "0.3.3", implementation: $impl, factory: $factory, metaFactory: $meta },
      webAuthnValidator: $webauthn,
      erc6492Verifier: $v6492,
      contracts: {
        MockUSDT: $usdt, EscrowConfig: $config, Escrow: $escrow, StreamEscrow: $stream,
        FixedFeePaymaster: $paymaster
      },
      paymasterSigner: $signer,
      deployer: $deployer
    }')
  if [[ "${AA_WRITE_CHAIN_JSON:-}" == 1 ]]; then
    printf '%s\n' "$expected" >"$CHAIN_JSON"
    log "已改写 ${CHAIN_JSON#"$ROOT/"}（记得提交）"
  elif ! diff -u <(jq -S . "$CHAIN_JSON") <(jq -S . <<<"$expected") >&2; then
    die "${CHAIN_JSON#"$ROOT/"} 与本次部署不符（上面是差异）。确认是有意的改动后运行 AA_WRITE_CHAIN_JSON=1 $0 up 并提交"
  else
    log "  ✓ ${CHAIN_JSON#"$ROOT/"} 与部署一致"
  fi
  if [[ "$ANVIL_PORT" != "$DEFAULT_ANVIL_PORT" || "$BUNDLER_PORT" != "$DEFAULT_BUNDLER_PORT" ]]; then
    log "注意：非默认端口只写进后端配置；钱包前端与 e2e 读 chain.local.json，仍连 $DEFAULT_ANVIL_PORT / $DEFAULT_BUNDLER_PORT"
  fi
}

# 宿主机网络下端口必须只监听本机回环地址。
check_loopback() {
  local port=$1 name=$2 addrs
  addrs=$(ss -ltnH "sport = :$port" | awk '{print $4}')
  [[ -n "$addrs" ]] || die "$name 没有在端口 $port 上监听"
  if grep -qv '^127\.0\.0\.1:' <<<"$addrs"; then
    die "$name 监听了本机回环以外的地址：$(tr '\n' ' ' <<<"$addrs")"
  fi
  log "  ✓ $name 只监听 127.0.0.1:$port"
}

# 后端链配置：地址与 chain.local.json 相同；钱包初始化参数第 1 版与前端 localChain 的版本 1 逐项一致
# （frontend/packages/contracts/src/chain.ts），否则钱包页面会因参数与服务器不一致而拒绝操作。
write_backend_config() {
  local start_block=$1 usdt=$2 config=$3 escrow=$4 stream=$5 paymaster=$6
  cat >"$CHAIN_TOML" <<TOML
# 由 deploy/dev/aa.sh 生成，勿手改。本地 AA 开发链，仅限开发。
chain_id = $CHAIN_ID
name = "Akari 本地链（Arbitrum Sepolia fork）"

[rpc.primary]
http = "$RPC"
ws = "${RPC/http:/ws:}"

[contracts]
escrow = "$escrow"
stream = "$stream"
config = "$config"
# 部署前的最新块：indexer 首次启动从这里补拉（fork 来的区块号很大，不能从 0 开始）。
start_block = $start_block

[token]
symbol = "mUSDT"
address = "$usdt"
decimals = 6

[aa]
entry_point = "$ENTRY_POINT"
paymaster = "$paymaster"
factories = ["$KERNEL_META_FACTORY"]

# anvil 每秒一块，safe / finalized 标记 = 最新块往前 32 / 64 块。
[indexer]
reconcile_interval_ms = 1000

[[wallet_init_versions]]
version = 1
kernel_implementation = "$KERNEL_IMPL"
kernel_factory = "$KERNEL_FACTORY"
kernel_meta_factory = "$KERNEL_META_FACTORY"
webauthn_validator = "$WEBAUTHN_VALIDATOR"
entry_point = "$ENTRY_POINT"
paymaster = "$paymaster"
approve_token = "$usdt"
approve_amount = "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
TOML
  cat >"$DEV_ENV" <<ENV
# 由 deploy/dev/aa.sh 生成，勿手改；仅限开发（私钥由公开字符串派生）。用法见 deploy/dev/README-aa.md。
CHAIN_CONFIG=$CHAIN_TOML
SPONSOR_URL=http://127.0.0.1:$SPONSOR_PORT
SPONSOR_LISTEN=127.0.0.1:$SPONSOR_PORT
SPONSOR_API_TOKEN=$SPONSOR_API_TOKEN
SPONSOR_DEV_PAYMASTER_KEY=$SPONSOR_KEY
SPONSOR_DEV_HOT_WALLET_KEY=$RELAYER_KEY
SPONSOR_DEV_AUCTION_KEY=$AUCTION_KEY
FALLBACK_BUNDLER_URL=$BUNDLER
ENV
  log "已写入 ${CHAIN_TOML#"$ROOT/"}、${DEV_ENV#"$ROOT/"}"
}

down() {
  "${COMPOSE[@]}" rm -sf alto anvil
  rm -f "$CHAIN_TOML" "$DEV_ENV" # 链状态不保留，部署区块随之失效
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
