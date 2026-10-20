#!/usr/bin/env bash
# Arbitrum Sepolia 测试网部署（plan 第 13、16 节）。完整步骤见 docs/runbooks/testnet-deploy.md。
#   deploy/testnet/testnet.sh preflight          只读检查：RPC、链 ID、AA 合约、Safe、部署账户余额、签名者地址
#   deploy/testnet/testnet.sh deploy [--resume]  forge script：MockUSDT、EscrowConfig / Escrow / StreamEscrow、FixedFeePaymaster
#   deploy/testnet/testnet.sh configure          部署账户补 Paymaster 押金与代发热钱包 ETH；Safe 才能做的写成 Transaction Builder 文件
#   deploy/testnet/testnet.sh verify             链上断言（角色、上限、费率、排队延迟、签名者、Paymaster）+ 代码与本地构建一致
#   deploy/testnet/testnet.sh emit-config        写后端链配置 TOML 与前端链配置 JSON（只有地址，不含密钥）
# 配置：TESTNET_ENV（默认 deploy/testnet/.env，模板 .env.example）。每一步都可重复运行：只做链上还没做到的部分。
# 密钥：RPC / Arbiscan 密钥只从 RPC_SECRETS_FILE（0600）读，经环境变量交给 forge / cast；部署账户用 cast wallet
# 密钥库或 Ledger。所有输出经过遮盖（密钥与带密钥的 URL 显示为 <redacted>），脚本不把它们写进任何文件。
set -Eeuo pipefail
trap 'printf "\033[1;31m[testnet]\033[0m 第 %s 行出错：%s\n" "$LINENO" "$BASH_COMMAND" >&2' ERR

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
CONTRACTS="$ROOT/contracts"
LOG_TAG=testnet
# shellcheck source-path=SCRIPTDIR source=../lib.sh
source "$ROOT/deploy/lib.sh"

CHAIN_ID=421614
CHAIN_NAME="Arbitrum Sepolia"
# 前端链配置的 rpcUrl / bundlerUrl 只填公共地址（不带密钥）：正式钱包读链与提交都经 forum-api，这两项只在开发模式直连时用。
PUBLIC_RPC=https://sepolia-rollup.arbitrum.io/rpc
PUBLIC_BUNDLER=https://public.pimlico.io/v2/421614/rpc
MAX_UINT256=115792089237316195423570985008687907853269984665640564039457584007913129639935
# 部署估算：五个合约约 1500 万 gas（L2 执行 + L1 数据费），按两倍留余量。
DEPLOY_GAS=30000000
# Deploy.s.sol defaultParams()（plan 第 9 节）：
# feeBps minFee minOrder orderCap totalLockedCap baseGas claimGas disputeMulBps resolveTimeout proposalWindow
EXPECTED_PARAMS="100 1000000 5000000 200000000 5000000000 100000 20000 10000 2592000 259200"
TIMELOCK_SECS=259200 # EscrowConfig.DELAY：调高收费 / 收紧规则 72 小时后生效
SAFE_TX_BUILDER_VERSION=1.18.0 # 与 backend/crates/chain/src/arbitration.rs 相同

export PATH="$HOME/.foundry/bin:$PATH"
export NO_PROXY="${NO_PROXY:+$NO_PROXY,}127.0.0.1,localhost,::1" no_proxy="${no_proxy:+$no_proxy,}127.0.0.1,localhost,::1"

FAILS=0
ok() { log "  ✓ $*"; }
bad() { printf '\033[1;31m[%s]\033[0m   ✗ %s\n' "$LOG_TAG" "$*" >&2; FAILS=$((FAILS + 1)); }
warn() { printf '\033[1;33m[%s]\033[0m   ! %s\n' "$LOG_TAG" "$*" >&2; }
expect() { # expect <描述> <实际> <期望>（地址不分大小写）
  if [[ "${2,,}" == "${3,,}" ]]; then ok "$1"; else bad "$1：链上 $2，应为 $3"; fi
}
finish() { # finish <步骤名>：有失败项就以非零退出
  if ((FAILS > 0)); then die "$1：$FAILS 项未通过"; fi
  log "$1：全部通过"
}

# ───────────────────────────── 配置与密钥 ─────────────────────────────

# 遮盖：把待遮盖的值（换行分隔，经环境变量传给 perl，不进命令行参数）替换成 <redacted>。
REDACT=""
add_secret() {
  [[ -n "$1" ]] || return 0
  local s=${1%/} tail
  REDACT+="$s"$'\n'
  # URL 最后一段（Infura / Alchemy 的密钥）单独遮盖，防止错误信息只带出这一段。
  tail=${s##*/}
  if [[ "$s" == *://*/* && ${#tail} -ge 16 ]]; then REDACT+="$tail"$'\n'; fi
}
redact() {
  AKARI_REDACT=$REDACT perl -pe 'BEGIN { $| = 1; @s = sort { length($b) <=> length($a) } grep { length > 7 } split /\n/, $ENV{AKARI_REDACT} } for my $s (@s) { s/\Q$s\E/<redacted>/g }'
}

abspath() { if [[ "$1" == /* ]]; then printf '%s' "$1"; else printf '%s/%s' "$ROOT" "$1"; fi; }

load_config() {
  local env=${TESTNET_ENV:-$HERE/.env}
  [[ -f "$env" ]] || die "缺少 $env（从 deploy/testnet/.env.example 复制后填写）"
  if grep -Eq '^[[:space:]]*(export[[:space:]]+)?[A-Z0-9_]*(KEY|PASSWORD|SECRET|MNEMONIC)=[^[:space:]#]' "$env"; then
    die "$env 里有密钥赋值；RPC / Arbiscan 密钥只放 RPC_SECRETS_FILE，部署私钥只放密钥库"
  fi
  if grep -E '^[^#]*://' "$env" |
    grep -Evq '^[[:space:]]*(export[[:space:]]+)?RPC(_BACKUP)?_URL=https?://(127\.0\.0\.1|localhost)(:[0-9]+)?/?[[:space:]]*$'; then
    die "$env 里有远程 URL；带密钥的 RPC 地址只放 RPC_SECRETS_FILE"
  fi
  set -a
  # shellcheck disable=SC1090
  source "$env"
  set +a
  STATE_DIR=$(abspath "${STATE_DIR:-deploy/testnet/deployments/arbitrum-sepolia}")
  FRONTEND_CHAIN_JSON=$(abspath "${FRONTEND_CHAIN_JSON:-frontend/packages/contracts/src/chain.arbitrum-sepolia.json}")
  BACKEND_CHAIN_TOML="$STATE_DIR/chain.toml"
  DEPLOYMENT="$STATE_DIR/deployment.json"
  BROADCAST="$STATE_DIR/broadcast"
  local v
  for v in ARBITER_SAFE ARBITER_SAFE_OWNERS TREASURY_SAFE TREASURY_SAFE_OWNERS OPS_SAFE OPS_SAFE_OWNERS \
    PAYMASTER_SIGNER RELAYER_ADDRESS AUCTION_SIGNER TOKEN; do
    printf -v "$v" '%s' "${!v:-}" # 未填的项按空值处理（preflight 逐项报告）
  done
  ARBITER_SAFE_THRESHOLD=${ARBITER_SAFE_THRESHOLD:-2}
  TREASURY_SAFE_THRESHOLD=${TREASURY_SAFE_THRESHOLD:-2}
  OPS_SAFE_THRESHOLD=${OPS_SAFE_THRESHOLD:-1}
  PAYMASTER_DEPOSIT=${PAYMASTER_DEPOSIT:-0.1}
  RELAYER_FUND=${RELAYER_FUND:-0.02}
  SPONSOR_ALERT_DEPOSIT=${SPONSOR_ALERT_DEPOSIT:-0.05}
  SPONSOR_ALERT_HOT_WALLET=${SPONSOR_ALERT_HOT_WALLET:-0.01}
  PAYMASTER_STAKE=${PAYMASTER_STAKE:-0.01}
  PAYMASTER_UNSTAKE_DELAY=${PAYMASTER_UNSTAKE_DELAY:-86400}
  INDEXER_RECONCILE_MS=${INDEXER_RECONCILE_MS:-5000}
}

# RPC：RPC_URL（只允许本机，fork 演练用）或 RPC_SECRETS_FILE 里的主备节点。
load_rpc() {
  ARBISCAN_API_KEY=""
  if [[ -n "${RPC_URL:-}" ]]; then
    FORK=1
    PRIMARY=$RPC_URL
    BACKUP=${RPC_BACKUP_URL:-}
    WS=""
  else
    FORK=0
    local f
    f=$(abspath "${RPC_SECRETS_FILE:?RPC_SECRETS_FILE 未设置（见 .env.example）}")
    private_file "$f" RPC_SECRETS_FILE
    PRIMARY=$(secret_value "$f" ARBITRUM_SEPOLIA_RPC_URL)
    BACKUP=$(secret_value "$f" ARBITRUM_SEPOLIA_RPC_BACKUP_URL)
    WS=$(secret_value "$f" ARBITRUM_SEPOLIA_WS_URL)
    ARBISCAN_API_KEY=$(secret_value "$f" ARBISCAN_API_KEY)
    [[ "$PRIMARY" == https://* ]] || die "RPC_SECRETS_FILE 缺 ARBITRUM_SEPOLIA_RPC_URL（https://…）"
    [[ -z "$BACKUP" || "$BACKUP" == https://* ]] || die "ARBITRUM_SEPOLIA_RPC_BACKUP_URL 必须是 https://…"
    local s
    for s in "$PRIMARY" "$BACKUP" "$WS" "$ARBISCAN_API_KEY"; do add_secret "$s"; done
    export ARBITRUM_SEPOLIA_RPC_URL=$PRIMARY # foundry.toml 的 rpc_endpoints.arbitrum_sepolia
    if [[ -n "$ARBISCAN_API_KEY" ]]; then export ARBISCAN_API_KEY; fi
  fi
  export ETH_RPC_URL=$PRIMARY
}

# 部署账户：cast wallet 密钥库或 Ledger，绝不接受明文私钥。
load_wallet() {
  [[ "${DEPLOYER_ADDRESS:-}" =~ ^0x[0-9a-fA-F]{40}$ ]] || die "DEPLOYER_ADDRESS 未设置或格式错误"
  if [[ "${DEPLOYER_LEDGER:-}" == 1 ]]; then
    WALLET=(--ledger)
  elif [[ -n "${DEPLOYER_KEYSTORE:-}" ]]; then
    private_file "$(abspath "$DEPLOYER_KEYSTORE")" DEPLOYER_KEYSTORE
    WALLET=(--keystore "$(abspath "$DEPLOYER_KEYSTORE")")
  elif [[ -n "${DEPLOYER_ACCOUNT:-}" ]]; then
    WALLET=(--account "$DEPLOYER_ACCOUNT")
  else
    die "设置 DEPLOYER_ACCOUNT、DEPLOYER_KEYSTORE 或 DEPLOYER_LEDGER=1 之一"
  fi
  if [[ -n "${DEPLOYER_PASSWORD_FILE:-}" ]]; then
    private_file "$(abspath "$DEPLOYER_PASSWORD_FILE")" DEPLOYER_PASSWORD_FILE
    WALLET+=(--password-file "$(abspath "$DEPLOYER_PASSWORD_FILE")")
  fi
}

host_of() { local h=${1#*://}; h=${h%%/*}; printf '%s' "${h##*@}"; }

# ───────────────────────────── 链上读取小工具 ─────────────────────────────

call() { cast call "$@" | awk '{print $1}'; } # 单个返回值（去掉 cast 附加的科学计数注释）
tuple() { tr -d '(),' | sed 's/ \[[^]]*\]//g'; } # cast 打印的元组 → 空格分隔的值
deposit_info() { # EntryPoint.getDepositInfo(paymaster) → deposit staked stake unstakeDelaySec withdrawTime
  cast call "$ENTRY_POINT" 'getDepositInfo(address)((uint256,bool,uint112,uint32,uint48))' "$1" | tuple
}
to_wei() { cast to-wei "$1"; }
wei_lt() { # wei_lt a b：任意大小的十进制整数比较
  local a=${1#"${1%%[!0]*}"} b=${2#"${2%%[!0]*}"}
  ((${#a} != ${#b})) && { ((${#a} < ${#b})); return; }
  [[ "$a" < "$b" ]]
}
eth() { cast from-wei "$1"; }

# 原始 JSON-RPC（curl 从标准输入读 URL，URL 不进命令行参数）。打印响应体；第 3 个参数给出时把耗时（秒）写进该变量。
rpc_raw() {
  local url=$1 body=$2 out
  out=$(printf 'url = "%s"\n' "$url" |
    curl -sS --max-time 30 -K - -H 'content-type: application/json' --data-binary "$body" -w '\n%{time_total}') || return 1
  if [[ -n "${3:-}" ]]; then printf -v "$3" '%s' "${out##*$'\n'}"; fi
  printf '%s' "${out%$'\n'*}"
}

load_deployment() {
  [[ -f "$DEPLOYMENT" ]] || die "没有部署记录 $DEPLOYMENT（先运行 deploy）"
  read -r TOKEN_ADDR CONFIG ESCROW STREAM PAYMASTER START_BLOCK DEPLOYED_BY TOKEN_DEPLOYED < <(jq -r \
    '[.token, .contracts.EscrowConfig, .contracts.Escrow, .contracts.StreamEscrow, .contracts.FixedFeePaymaster,
      .startBlock, .deployer, (.contracts.MockUSDT != null)] | @tsv' "$DEPLOYMENT")
  [[ "$(jq -r .chainId "$DEPLOYMENT")" == "$CHAIN_ID" ]] || die "$DEPLOYMENT 不是链 $CHAIN_ID 的部署"
}

# ───────────────────────────── preflight ─────────────────────────────

check_endpoint() { # check_endpoint <名称> <URL>：链 ID、延迟、eth_simulateV1、NodeInterface、eth_getLogs 范围
  local name=$1 url=$2 resp t total=0 head
  name="$name（$(host_of "$url")）"
  if ! resp=$(rpc_raw "$url" '{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}'); then
    bad "$name 连不上"
    return
  fi
  local id
  id=$(jq -r '.result // empty' <<<"$resp" 2>/dev/null) || id=""
  expect "$name 链 ID" "$(if [[ -n "$id" ]]; then cast to-dec "$id"; else echo "?"; fi)" "$CHAIN_ID"
  for _ in 1 2 3; do
    rpc_raw "$url" '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}' t >/dev/null || { bad "$name eth_blockNumber 失败"; return; }
    total=$(awk -v a="$total" -v b="$t" 'BEGIN { print a + b }')
  done
  ok "$name 延迟 $(awk -v t="$total" 'BEGIN { printf "%.0f", t / 3 * 1000 }') ms（eth_blockNumber 3 次平均）"

  # forum-sponsor 首选 eth_simulateV1，节点崩溃（Nitro 的 -32603）或不支持时自动改用 EntryPoint eth_call（只是记录）。
  local data
  data=$(cast calldata 'balanceOf(address)' 0x000000000000000000000000000000000000dEaD)
  resp=$(rpc_raw "$url" "$(jq -cn --arg to "$ENTRY_POINT" --arg data "$data" '{jsonrpc:"2.0",id:1,method:"eth_simulateV1",
    params:[{blockStateCalls:[{calls:[{from:"0x000000000000000000000000000000000000dEaD",to:$to,data:$data}]}],
      traceTransfers:false,validation:false,returnFullTransactions:false},"latest"]}')") || resp='{"error":{"message":"request failed"}}'
  if jq -e '.result[0].calls[0].status == "0x1"' <<<"$resp" >/dev/null 2>&1; then
    ok "$name eth_simulateV1 可用"
  else
    warn "$name eth_simulateV1 不可用：$(jq -c '.error // .' <<<"$resp" | cut -c1-200)（forum-sponsor 改用 EntryPoint eth_call 模拟）"
  fi

  # forum-sponsor 用 NodeInterface.gasEstimateL1Component 估 L1 数据费（Arbitrum 的虚拟合约，0xC8）。
  data=$(cast calldata 'gasEstimateL1Component(address,bool,bytes)' "$ENTRY_POINT" false 0x)
  resp=$(rpc_raw "$url" "$(jq -cn --arg data "$data" '{jsonrpc:"2.0",id:1,method:"eth_call",
    params:[{to:"0x00000000000000000000000000000000000000C8",data:$data},"latest"]}')") || resp='{}'
  if jq -e '(.result // "0x") | length > 2' <<<"$resp" >/dev/null 2>&1; then
    ok "$name NodeInterface.gasEstimateL1Component 可用"
  else
    bad "$name NodeInterface.gasEstimateL1Component 失败：$(jq -c '.error // .' <<<"$resp" | cut -c1-200)"
  fi

  # forum-indexer 每次 eth_getLogs 最多跨 2000 块（[indexer] max_block_range）。
  head=$(cast to-dec "$(rpc_raw "$url" '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}' | jq -r .result)")
  resp=$(rpc_raw "$url" "$(jq -cn --arg from "$(cast to-hex $((head - 1999)))" --arg to "$(cast to-hex "$head")" \
    --arg addr "$ENTRY_POINT" '{jsonrpc:"2.0",id:1,method:"eth_getLogs",params:[{fromBlock:$from,toBlock:$to,address:$addr}]}')") || resp='{}'
  if jq -e '.result | type == "array"' <<<"$resp" >/dev/null 2>&1; then
    ok "$name eth_getLogs 跨 2000 块可用（EntryPoint 日志 $(jq '.result | length' <<<"$resp") 条）"
  else
    bad "$name eth_getLogs 跨 2000 块失败：$(jq -c '.error // .' <<<"$resp" | cut -c1-200)（调小 [indexer] max_block_range）"
  fi
}

norm_list() { tr -d '[]" ' <<<"$1" | tr ',' '\n' | sed '/^$/d' | tr '[:upper:]' '[:lower:]' | sort | paste -sd, -; }

check_safe() { # check_safe <名称> <地址> <期望 owners> <期望门槛>
  local name=$1 safe=$2 want=$3 threshold=$4 got_t got_o version
  if [[ ! "$safe" =~ ^0x[0-9a-fA-F]{40}$ ]]; then bad "$name 地址未设置或格式错误"; return; fi
  if ! has_code "$safe"; then bad "$name $safe 没有代码（Safe 还没建，或地址填错了链）"; return; fi
  if ! got_t=$(call "$safe" 'getThreshold()(uint256)' 2>/dev/null) ||
    ! got_o=$(cast call "$safe" 'getOwners()(address[])' 2>/dev/null); then
    bad "$name $safe 不是 Safe（getThreshold / getOwners 失败）"
    return
  fi
  version=$(cast call "$safe" 'VERSION()(string)' 2>/dev/null | tr -d '"') || version="?"
  [[ -n "$want" ]] || { bad "$name 的 OWNERS 未配置（链上为 $(norm_list "$got_o")）"; return; }
  if [[ "$got_t" == "$threshold" && "$(norm_list "$got_o")" == "$(norm_list "$want")" ]]; then
    ok "$name $safe（Safe $version，$threshold/$(norm_list "$got_o" | tr ',' '\n' | wc -l)，签名人与配置一致）"
  else
    bad "$name $safe：链上 $got_t/[$(norm_list "$got_o")]，配置 $threshold/[$(norm_list "$want")]"
  fi
  if [[ ",$(norm_list "$got_o")," == *",${DEPLOYER_ADDRESS,,},"* ]]; then bad "$name 的签名人里有部署账户"; fi
}

check_roles() { # Safe 与签名者：Safe 互不相同；热密钥互不相同，也不是部署账户或 Safe
  check_safe "仲裁 Safe" "$ARBITER_SAFE" "$ARBITER_SAFE_OWNERS" "$ARBITER_SAFE_THRESHOLD"
  check_safe "金库 Safe" "$TREASURY_SAFE" "$TREASURY_SAFE_OWNERS" "$TREASURY_SAFE_THRESHOLD"
  check_safe "运营 Safe" "$OPS_SAFE" "$OPS_SAFE_OWNERS" "$OPS_SAFE_THRESHOLD"
  if [[ -n "$ARBITER_SAFE" && "${ARBITER_SAFE,,}" == "${TREASURY_SAFE,,}" ]]; then bad "仲裁 Safe 不能是金库 Safe（ADR 0013）"; fi
  local name addr seen=" ${DEPLOYER_ADDRESS,,} ${ARBITER_SAFE,,} ${TREASURY_SAFE,,} ${OPS_SAFE,,} "
  for name in PAYMASTER_SIGNER RELAYER_ADDRESS AUCTION_SIGNER; do
    addr=${!name:-}
    if [[ -z "$addr" && "$name" == AUCTION_SIGNER ]]; then warn "AUCTION_SIGNER 未设置：拍卖关闭"; continue; fi
    if [[ ! "$addr" =~ ^0x[0-9a-fA-F]{40}$ ]]; then bad "$name 未设置或格式错误"; continue; fi
    if [[ "$seen" == *" ${addr,,} "* ]]; then bad "$name $addr 与部署账户 / Safe / 其他签名者重复"; continue; fi
    seen+="${addr,,} "
    if has_code "$addr"; then bad "$name $addr 有代码（应是 forum-sponsor 持有私钥的 EOA）"; else ok "$name $addr"; fi
  done
}

check_aa_code() {
  check_code_soft EntryPoint-v0.7 "$ENTRY_POINT"
  check_code_soft Kernel-v3.3-implementation "$KERNEL_IMPL"
  check_code_soft Kernel-v3.3-factory "$KERNEL_FACTORY"
  check_code_soft Kernel-v3.3-meta-factory "$KERNEL_META_FACTORY"
  check_code_soft WebAuthn-validator-0.0.3 "$WEBAUTHN_VALIDATOR"
  check_code_soft CREATE2-deployer "$CREATE2_DEPLOYER"
  if has_code "$ERC6492_VERIFIER"; then ok "ERC-6492-verifier $ERC6492_VERIFIER"; else warn "ERC-6492 验证器不在链上，deploy 时经 CREATE2 部署"; fi
  # 后端推算钱包地址时同样核对这一项（backend/crates/chain/src/kernel.rs）。
  expect "Kernel 工厂的实现是 Kernel v3.3" "$(cast call "$KERNEL_FACTORY" 'implementation()(address)')" "$KERNEL_IMPL"
  if p256_ok; then ok "secp256r1 预编译 0x100（RIP-7212）"; else bad "secp256r1 预编译 0x100 不可用"; fi
}

check_code_soft() { if has_code "$2"; then ok "$1 $2"; else bad "$1 $2 没有代码"; fi; }

preflight() {
  log "preflight：$CHAIN_NAME（$([[ $FORK == 1 ]] && echo "本机 fork $PRIMARY" || echo "RPC_SECRETS_FILE")）"
  local tool
  for tool in forge cast jq curl perl; do command -v "$tool" >/dev/null || bad "找不到 $tool"; done

  log "RPC 节点"
  check_endpoint 主节点 "$PRIMARY"
  if [[ -n "$BACKUP" ]]; then check_endpoint 备用节点 "$BACKUP"; else warn "没有备用节点（ARBITRUM_SEPOLIA_RPC_BACKUP_URL）：plan 第 2 节要求每类至少两家，测试网可暂缺"; fi
  if [[ -n "$WS" ]]; then
    expect "主节点 WebSocket 链 ID" "$(ETH_RPC_URL=$WS cast chain-id 2>/dev/null || echo 连不上)" "$CHAIN_ID"
  fi
  if [[ "$(cast chain-id)" != "$CHAIN_ID" ]]; then finish preflight; fi # 链不对，后面的检查没有意义

  log "AA 基础合约"
  check_aa_code

  log "Safe 与签名者"
  check_roles

  log "代币"
  if [[ -n "$TOKEN" ]]; then
    check_code_soft TOKEN "$TOKEN"
    expect "TOKEN 小数位" "$(call "$TOKEN" 'decimals()(uint8)' 2>/dev/null || echo "?")" 6
  else
    ok "TOKEN 留空：deploy 部署 MockUSDT（测试网专用，任何人可 mint）"
  fi

  log "部署账户 $DEPLOYER_ADDRESS"
  local balance gas_price need
  balance=$(cast balance "$DEPLOYER_ADDRESS")
  gas_price=$(cast gas-price)
  need=$(($(to_wei "$PAYMASTER_DEPOSIT") + $(to_wei "$RELAYER_FUND") + DEPLOY_GAS * gas_price))
  if has_code "$DEPLOYER_ADDRESS"; then bad "部署账户有代码（应是 EOA）"; fi
  if wei_lt "$balance" "$need"; then
    bad "部署账户余额 $(eth "$balance") ETH，部署 + 押金 + 热钱包约需 $(eth "$need") ETH（gas 价 $gas_price wei）"
  else
    ok "余额 $(eth "$balance") ETH ≥ 约需 $(eth "$need") ETH；nonce $(cast nonce "$DEPLOYER_ADDRESS")"
  fi
  if [[ "${DEPLOYER_LEDGER:-}" != 1 && -n "${DEPLOYER_ACCOUNT:-}" && -z "${DEPLOYER_KEYSTORE:-}" &&
    ! -f "${FOUNDRY_KEYSTORES_DIR:-$HOME/.foundry/keystores}/$DEPLOYER_ACCOUNT" ]]; then
    bad "密钥库 $DEPLOYER_ACCOUNT 不存在（cast wallet import $DEPLOYER_ACCOUNT --interactive）"
  fi
  if [[ -n "$OPS_SAFE" ]] && wei_lt "$(cast balance "$OPS_SAFE")" "$(to_wei "$PAYMASTER_STAKE")"; then
    warn "运营 Safe 余额不足 $PAYMASTER_STAKE ETH：configure 生成的质押交易要从它转出"
  fi

  # forum-sponsor 低于告警线就告警，热钱包低于告警线还视为代发器不健康（改走备用 bundler）。
  if ! wei_lt "$(to_wei "$SPONSOR_ALERT_DEPOSIT")" "$(to_wei "$PAYMASTER_DEPOSIT")"; then
    bad "PAYMASTER_DEPOSIT $PAYMASTER_DEPOSIT ETH 不高于 forum-sponsor 押金告警线 $SPONSOR_ALERT_DEPOSIT ETH"
  fi
  if ! wei_lt "$(to_wei "$SPONSOR_ALERT_HOT_WALLET")" "$(to_wei "$RELAYER_FUND")"; then
    bad "RELAYER_FUND $RELAYER_FUND ETH 不高于 forum-sponsor 热钱包告警线 $SPONSOR_ALERT_HOT_WALLET ETH（低于它代发器视为不健康）"
  fi

  if [[ $FORK == 0 && -z "$ARBISCAN_API_KEY" ]]; then warn "没有 ARBISCAN_API_KEY：deploy 不验证源码（plan 第 16 节要求验证）"; fi
  finish preflight
}

# ───────────────────────────── deploy ─────────────────────────────

deploy() {
  local resume=0
  if [[ "${1:-}" == --resume ]]; then resume=1; fi
  if [[ -f "$DEPLOYMENT" ]]; then
    die "已有部署记录 $DEPLOYMENT；要重新部署，按 docs/runbooks/testnet-deploy.md「重新部署」先归档旧记录"
  fi
  if ((resume == 0)); then
    [[ ! -d "$BROADCAST" ]] || die "$BROADCAST 已存在但没有部署记录：上次中断了？用 deploy --resume 续发，或归档该目录后重来"
    preflight
  fi
  (cd "$CONTRACTS" && forge build --quiet)

  local args=(script/Deploy.s.sol --broadcast --slow --sender "$DEPLOYER_ADDRESS" "${WALLET[@]}")
  if [[ $FORK == 1 ]]; then
    args+=(--rpc-url "$PRIMARY")
  else
    args+=(--rpc-url arbitrum_sepolia) # URL 取自环境变量 ARBITRUM_SEPOLIA_RPC_URL（foundry.toml）
    if [[ -n "$ARBISCAN_API_KEY" ]]; then args+=(--verify); fi
  fi
  if ((resume)); then args+=(--resume); fi
  local envs=(OWNER="$TREASURY_SAFE" ARBITER="$ARBITER_SAFE" FEE_RECIPIENT="$TREASURY_SAFE"
    PAYMASTER_SIGNER="$PAYMASTER_SIGNER" PAYMASTER_OWNER="$OPS_SAFE" FOUNDRY_BROADCAST="$BROADCAST")
  if [[ -n "$TOKEN" ]]; then envs+=(TOKEN="$TOKEN"); fi
  if [[ -n "$AUCTION_SIGNER" ]]; then envs+=(AUCTION_SIGNER="$AUCTION_SIGNER"); fi
  mkdir -p "$STATE_DIR"
  log "forge script Deploy.s.sol（部署账户 $DEPLOYER_ADDRESS$([[ " ${args[*]} " == *" --verify "* ]] && echo "，Arbiscan 验证源码")）"
  # forge 把每笔交易用的 RPC 地址（含密钥）记在 contracts/cache/Deploy.s.sol/ 里供 --resume 用：只留给本人，成功后删掉。
  local sensitive="$CONTRACTS/cache/Deploy.s.sol/$CHAIN_ID" rc=0
  (cd "$CONTRACTS" && env -u TOKEN -u AUCTION_SIGNER "${envs[@]}" forge script "${args[@]}") || rc=$?
  if [[ -d "$sensitive" ]]; then chmod -R go-rwx "$CONTRACTS/cache/Deploy.s.sol"; fi
  ((rc == 0)) || die "forge script 失败（$rc）；修好后用 deploy --resume 续发"
  record_deployment
  rm -rf "$sensitive"
}

# 从 forge 的广播记录整理出部署记录（地址、部署区块、部署账户）。
record_deployment() {
  local run="$BROADCAST/Deploy.s.sol/$CHAIN_ID/run-latest.json" blocks start
  [[ -f "$run" ]] || die "找不到广播记录 $run"
  if jq -e --arg d "${DEPLOYER_ADDRESS,,}" 'any(.transactions[]; (.transaction.from | ascii_downcase) != $d)' "$run" >/dev/null; then
    die "广播记录里有不是 DEPLOYER_ADDRESS 发出的交易（密钥库 / Ledger 账户与 DEPLOYER_ADDRESS 不符？）"
  fi
  jq -e '(.receipts | length) == (.transactions | length) and all(.receipts[]; .status == "0x1")' "$run" >/dev/null ||
    die "广播未全部上链成功（deploy --resume 续发）"
  blocks=$(jq -r '.receipts[].blockNumber' "$run")
  start=$(while read -r b; do cast to-dec "$b"; done <<<"$blocks" | sort -n | head -n1)
  jq --arg deployer "$DEPLOYER_ADDRESS" --arg token "$TOKEN" --argjson start "$start" --argjson chain "$CHAIN_ID" '
    ([.transactions[] | select(.transactionType == "CREATE") | {(.contractName): .contractAddress}] | add) as $c
    | {
        chainId: $chain,
        deployer: $deployer,
        startBlock: $start,
        token: (if $token == "" then $c.MockUSDT else $token end),
        contracts: ($c | with_entries(select(.key | IN("MockUSDT", "EscrowConfig", "Escrow", "StreamEscrow", "FixedFeePaymaster")))),
        transactions: [.receipts[] | .transactionHash]
      }' "$run" >"$DEPLOYMENT"
  jq -e '.contracts | has("EscrowConfig") and has("Escrow") and has("StreamEscrow") and has("FixedFeePaymaster")' \
    "$DEPLOYMENT" >/dev/null || die "广播记录里缺合约（$DEPLOYMENT）"
  # forge 记的地址是小写：统一改成校验和格式（前端与后端配置都照抄这里）。
  local name addr
  for name in $(jq -r '.contracts | keys[]' "$DEPLOYMENT"); do
    addr=$(cast to-check-sum-address "$(jq -r --arg n "$name" '.contracts[$n]' "$DEPLOYMENT")")
    jq --arg n "$name" --arg a "$addr" '.contracts[$n] = $a
      | if (.token | ascii_downcase) == ($a | ascii_downcase) then .token = $a else . end' "$DEPLOYMENT" >"$DEPLOYMENT.tmp"
    mv "$DEPLOYMENT.tmp" "$DEPLOYMENT"
  done
  log "已写入 ${DEPLOYMENT#"$ROOT/"}（起始区块 $start）："
  jq -r '.contracts | to_entries[] | "  \(.key)\t\(.value)"' "$DEPLOYMENT" >&2
}

# ───────────────────────────── configure ─────────────────────────────

send() { # send <描述> <cast send 参数…>：部署账户发交易，核对发送方与执行结果
  local what=$1 receipt; shift
  receipt=$(cast send --json "${WALLET[@]}" "$@")
  [[ "$(jq -r .status <<<"$receipt")" == 0x1 ]] || die "$what 失败（交易 $(jq -r .transactionHash <<<"$receipt")）"
  [[ "$(jq -r '.from | ascii_downcase' <<<"$receipt")" == "${DEPLOYER_ADDRESS,,}" ]] || die "$what 不是 DEPLOYER_ADDRESS 发出的"
  ok "$what（交易 $(jq -r .transactionHash <<<"$receipt")）"
}

safe_tx() { # safe_tx <to> <value wei> <函数签名> <参数…>：输出一笔 Transaction Builder 交易（一行 JSON）
  local to=$1 value=$2 sig=$3; shift 3
  jq -cn --arg to "$(cast to-check-sum-address "$to")" --arg value "$value" --arg data "$(cast calldata "$sig" "$@")" \
    '{to: $to, value: $value, data: $data, contractMethod: null, contractInputsValues: null}'
}

# Safe Transaction Builder 批量文件（与 forum-api 给仲裁签名人的文件同一格式）；没有交易时删掉旧文件。
safe_batch() { # safe_batch <文件名> <Safe> <标题> <说明> [交易 JSON…]
  local file="$STATE_DIR/safe-tx/$1" safe=$2 name=$3 desc=$4; shift 4
  if (($# == 0)); then
    rm -f "$file"
    ok "$name：无需 Safe 操作"
    return
  fi
  mkdir -p "$STATE_DIR/safe-tx"
  printf '%s\n' "$@" | jq -s --arg chain "$CHAIN_ID" --arg safe "$(cast to-check-sum-address "$safe")" \
    --arg name "$name" --arg desc "$desc" --arg v "$SAFE_TX_BUILDER_VERSION" --argjson now "$(date +%s000)" '{
      version: "1.0", chainId: $chain, createdAt: $now,
      meta: {name: $name, description: $desc, txBuilderVersion: $v, createdFromSafeAddress: $safe, createdFromOwnerAddress: ""},
      transactions: .
    }' >"$file"
  warn "$name：$# 笔交易待 Safe $safe 执行 → ${file#"$ROOT/"}"
  PENDING_SAFE=1
}

configure() {
  load_deployment
  PENDING_SAFE=0
  log "部署账户直接做（任何人都能调用的）"
  local have want
  have=$(call "$ENTRY_POINT" 'balanceOf(address)(uint256)' "$PAYMASTER")
  want=$(to_wei "$PAYMASTER_DEPOSIT")
  if wei_lt "$have" "$want"; then
    send "Paymaster 押金补到 $PAYMASTER_DEPOSIT ETH" "$PAYMASTER" 'deposit()' --value "$((want - have))"
  else
    ok "Paymaster 押金 $(eth "$have") ETH ≥ $PAYMASTER_DEPOSIT"
  fi
  have=$(cast balance "$RELAYER_ADDRESS")
  want=$(to_wei "$RELAYER_FUND")
  if wei_lt "$have" "$want"; then
    send "代发热钱包 $RELAYER_ADDRESS 补到 $RELAYER_FUND ETH" "$RELAYER_ADDRESS" --value "$((want - have))"
  else
    ok "代发热钱包 $(eth "$have") ETH ≥ $RELAYER_FUND"
  fi

  log "运营 Safe（Paymaster owner）"
  local ops=() notes=() staked stake delay
  read -r _ staked stake delay _ < <(deposit_info "$PAYMASTER")
  want=$(to_wei "$PAYMASTER_STAKE")
  if [[ "$staked" != true ]] || wei_lt "$stake" "$want" || ((delay < PAYMASTER_UNSTAKE_DELAY)); then
    local add=0
    if wei_lt "$stake" "$want"; then add=$((want - stake)); fi
    ops+=("$(safe_tx "$PAYMASTER" "$add" 'addStake(uint32)' "$PAYMASTER_UNSTAKE_DELAY")")
    notes+=("addStake($PAYMASTER_UNSTAKE_DELAY) 附 $(eth "$add") ETH，质押到 $PAYMASTER_STAKE ETH（公共 bundler 备用通道要求）")
  else
    ok "Paymaster 已质押 $(eth "$stake") ETH，解押延迟 ${delay}s"
  fi
  if [[ "$(cast call "$PAYMASTER" 'signer()(address)')" != "$(cast to-check-sum-address "$PAYMASTER_SIGNER")" ]]; then
    ops+=("$(safe_tx "$PAYMASTER" 0 'setSigner(address)' "$PAYMASTER_SIGNER")")
    notes+=("setSigner($PAYMASTER_SIGNER) 换代付签名者")
  fi
  local desc
  desc=$(printf '%s；' ${notes[@]+"${notes[@]}"})
  safe_batch paymaster-ops.json "$OPS_SAFE" "Akari 测试网：FixedFeePaymaster" "FixedFeePaymaster $PAYMASTER：${desc%；}" \
    ${ops[@]+"${ops[@]}"}

  log "金库 Safe（EscrowConfig owner）"
  local treasury=() current target key eta
  current=$(cast call "$CONFIG" 'auctionSigner()(address)')
  target=$(cast to-check-sum-address "${AUCTION_SIGNER:-0x0000000000000000000000000000000000000000}")
  if [[ "$current" != "$target" ]]; then
    # 换拍卖签名者要排队 72 小时：排队由金库 Safe 做，到期后任何人都能 apply（这里由部署账户执行）。
    key=$(cast call "$CONFIG" 'queueKey(uint8,bytes)(bytes32)' 3 "$(cast abi-encode 'f(address,address)' "$current" "$target")")
    eta=$(call "$CONFIG" 'queuedEta(bytes32)(uint64)' "$key")
    if ((eta == 0)); then
      treasury+=("$(safe_tx "$CONFIG" 0 'queueAuctionSigner(address)' "$target")")
    elif ((eta <= $(cast block --field timestamp latest))); then
      send "applyAuctionSigner($target)" "$CONFIG" 'applyAuctionSigner(address)' "$target"
    else
      warn "拍卖签名者 $current → $target 已排队，$(date -d "@$eta" '+%F %T') 后再运行 configure"
    fi
  else
    ok "拍卖签名者 $current"
  fi
  if [[ "$(cast call "$CONFIG" 'pendingOwner()(address)')" != 0x0000000000000000000000000000000000000000 ]]; then
    warn "EscrowConfig 有待接受的 owner 转移（pendingOwner），确认是否有意为之"
  fi
  safe_batch escrow-config.json "$TREASURY_SAFE" "Akari 测试网：EscrowConfig 拍卖签名者" \
    "EscrowConfig $CONFIG：queueAuctionSigner($target)，72 小时后 configure 自动 apply" \
    ${treasury[@]+"${treasury[@]}"}

  if ((PENDING_SAFE)); then
    log "下一步：app.safe.global → 对应 Safe → Apps → Transaction Builder → 拖入上面的 JSON → 签名并执行；然后再运行 configure / verify"
  else
    log "configure：链上已与配置一致"
  fi
}

# ───────────────────────────── verify ─────────────────────────────

# 链上代码与本地构建一致：两边都把 immutable 所在的字节清零后比较（immutable 是部署时写入的地址与域分隔符）。
code_matches() { # code_matches <合约名> <地址>
  local artifact="$CONTRACTS/out/$1.sol/$1.json" onchain
  onchain=$(cast code "$2")
  jq -e --arg code "$onchain" '
    def mask($refs): reduce $refs[] as $r (.; .[0:$r.start * 2] + ("0" * ($r.length * 2)) + .[($r.start + $r.length) * 2:]);
    [.deployedBytecode.immutableReferences // {} | .[][]] as $refs
    | (.deployedBytecode.object | ltrimstr("0x") | ascii_downcase | mask($refs))
      == ($code | ltrimstr("0x") | ascii_downcase | mask($refs))' "$artifact" >/dev/null
}

verify() {
  load_deployment
  (cd "$CONTRACTS" && forge build --quiet)
  log "verify：$CHAIN_NAME，部署记录 ${DEPLOYMENT#"$ROOT/"}"
  [[ "$(cast chain-id)" == "$CHAIN_ID" ]] || die "RPC 链 ID 不是 $CHAIN_ID"

  log "代码与本地构建一致（忽略 immutable）"
  local name addr
  for name in MockUSDT EscrowConfig Escrow StreamEscrow FixedFeePaymaster; do
    addr=$(jq -r --arg n "$name" '.contracts[$n] // empty' "$DEPLOYMENT")
    [[ -n "$addr" ]] || continue
    if code_matches "$name" "$addr"; then ok "$name $addr"; else bad "$name $addr 的代码与本地构建不一致"; fi
  done
  check_code_soft ERC-6492-verifier "$ERC6492_VERIFIER"

  log "EscrowConfig $CONFIG"
  expect "owner = 金库 Safe" "$(cast call "$CONFIG" 'owner()(address)')" "$TREASURY_SAFE"
  expect "没有待接受的 owner 转移" "$(cast call "$CONFIG" 'pendingOwner()(address)')" 0x0000000000000000000000000000000000000000
  local params
  params=$(cast call "$CONFIG" 'params()((uint16,uint64,uint64,uint64,uint64,uint64,uint64,uint32,uint32,uint32))' | tuple)
  expect "参数（费率 1%、最低手续费 1、最低订单 5、单笔上限 200、总额上限 5000 USDT、gas 费 0.1 / 0.02、争议费 1 倍、仲裁超时 30 天、和解 72 小时）" \
    "$params" "$EXPECTED_PARAMS"
  expect "没有排队中的参数" "$(call "$CONFIG" 'pendingEta()(uint64)')" 0
  expect "调整排队延迟 DELAY = 72 小时" "$(call "$CONFIG" 'DELAY()(uint256)')" "$TIMELOCK_SECS"
  expect "未暂停" "$(cast call "$CONFIG" 'paused()(bool)')" false
  expect "没有免费活动" "$(cast call "$CONFIG" 'campaign()((uint64,uint64,uint16,uint8,uint32))' | tuple)" "0 0 0 0 0"
  expect "代币已列入白名单" "$(cast call "$CONFIG" 'tokenAllowed(address)(bool)' "$TOKEN_ADDR")" true
  expect "Escrow 是市场" "$(cast call "$CONFIG" 'isMarket(address)(bool)' "$ESCROW")" true
  expect "StreamEscrow 是市场" "$(cast call "$CONFIG" 'isMarket(address)(bool)' "$STREAM")" true
  expect "拍卖签名者" "$(cast call "$CONFIG" 'auctionSigner()(address)')" "${AUCTION_SIGNER:-0x0000000000000000000000000000000000000000}"

  log "Escrow $ESCROW / StreamEscrow $STREAM"
  expect "Escrow.config" "$(cast call "$ESCROW" 'config()(address)')" "$CONFIG"
  expect "Escrow.arbiter = 仲裁 Safe" "$(cast call "$ESCROW" 'arbiter()(address)')" "$ARBITER_SAFE"
  expect "Escrow.feeRecipient = 金库 Safe" "$(cast call "$ESCROW" 'feeRecipient()(address)')" "$TREASURY_SAFE"
  expect "StreamEscrow.escrow" "$(cast call "$STREAM" 'escrow()(address)')" "$ESCROW"
  expect "StreamEscrow.config" "$(cast call "$STREAM" 'config()(address)')" "$CONFIG"
  expect "StreamEscrow.arbiter = 仲裁 Safe" "$(cast call "$STREAM" 'arbiter()(address)')" "$ARBITER_SAFE"
  expect "StreamEscrow.feeRecipient = 金库 Safe" "$(cast call "$STREAM" 'feeRecipient()(address)')" "$TREASURY_SAFE"

  log "FixedFeePaymaster $PAYMASTER"
  expect "entryPoint = EntryPoint v0.7" "$(cast call "$PAYMASTER" 'entryPoint()(address)')" "$ENTRY_POINT"
  expect "token" "$(cast call "$PAYMASTER" 'token()(address)')" "$TOKEN_ADDR"
  expect "treasury = 金库 Safe" "$(cast call "$PAYMASTER" 'treasury()(address)')" "$TREASURY_SAFE"
  expect "owner = 运营 Safe" "$(cast call "$PAYMASTER" 'owner()(address)')" "$OPS_SAFE"
  expect "signer = PAYMASTER_SIGNER" "$(cast call "$PAYMASTER" 'signer()(address)')" "$PAYMASTER_SIGNER"
  expect "MAX_FEE = 1 USDT" "$(call "$PAYMASTER" 'MAX_FEE()(uint256)')" 1000000
  expect "MAX_VALIDITY = 10 分钟" "$(call "$PAYMASTER" 'MAX_VALIDITY()(uint256)')" 600
  local deposit staked stake delay withdraw
  read -r deposit staked stake delay withdraw < <(deposit_info "$PAYMASTER")
  if wei_lt "$deposit" "$(to_wei "$PAYMASTER_DEPOSIT")"; then bad "押金 $(eth "$deposit") ETH < $PAYMASTER_DEPOSIT（运行 configure）"; else ok "押金 $(eth "$deposit") ETH"; fi
  if [[ "$staked" == true ]] && ! wei_lt "$stake" "$(to_wei "$PAYMASTER_STAKE")" &&
    ((delay >= PAYMASTER_UNSTAKE_DELAY && withdraw == 0)); then
    ok "质押 $(eth "$stake") ETH，解押延迟 ${delay}s"
  else
    bad "质押：staked=$staked stake=$(eth "$stake") ETH delay=${delay}s withdrawTime=$withdraw（运营 Safe 执行 configure 生成的交易）"
  fi
  local relayer
  relayer=$(cast balance "$RELAYER_ADDRESS")
  if wei_lt "$relayer" "$(to_wei "$RELAYER_FUND")"; then bad "代发热钱包 $(eth "$relayer") ETH < $RELAYER_FUND"; else ok "代发热钱包 $(eth "$relayer") ETH"; fi

  if [[ "$TOKEN_DEPLOYED" == true ]]; then
    log "MockUSDT $TOKEN_ADDR"
    expect "decimals" "$(call "$TOKEN_ADDR" 'decimals()(uint8)')" 6
    expect "symbol" "$(cast call "$TOKEN_ADDR" 'symbol()(string)' | tr -d '"')" mUSDT
  fi

  log "Safe 与签名者"
  check_roles
  log "AA 基础合约"
  check_aa_code
  finish verify
}

# ───────────────────────────── emit-config ─────────────────────────────

# 钱包初始化参数版本只增不改（ADR 0008）：已有前端链配置时沿用其中的版本；本次部署的 Paymaster / 代币与最新版本
# 不同（重新部署了 Paymaster 或换了代币）才追加一个新版本。
wallet_versions() {
  local current
  current=$(jq -n --arg impl "$KERNEL_IMPL" --arg factory "$KERNEL_FACTORY" --arg meta "$KERNEL_META_FACTORY" \
    --arg webauthn "$WEBAUTHN_VALIDATOR" --arg ep "$ENTRY_POINT" --arg pm "$PAYMASTER" --arg token "$TOKEN_ADDR" \
    --arg amount "$MAX_UINT256" '{kernelImpl: $impl, factory: $factory, metaFactory: $meta, webauthnValidator: $webauthn,
      entryPoint: $ep, paymaster: $pm, approveToken: $token, approveAmount: $amount}')
  if [[ ! -f "$FRONTEND_CHAIN_JSON" ]]; then
    jq -c '[{version: 1} + .]' <<<"$current"
    return
  fi
  [[ "$(jq -r .chainId "$FRONTEND_CHAIN_JSON")" == "$CHAIN_ID" ]] || die "$FRONTEND_CHAIN_JSON 不是链 $CHAIN_ID 的配置"
  jq -c --argjson cur "$current" '.walletInitVersions as $v
    | ($v[-1] | del(.version) | with_entries(.value |= ascii_downcase)) as $last
    | if $last == ($cur | with_entries(.value |= ascii_downcase)) then $v else $v + [{version: ($v[-1].version + 1)} + $cur] end' \
    "$FRONTEND_CHAIN_JSON"
}

emit_config() {
  load_deployment
  local versions
  versions=$(wallet_versions)
  if [[ -f "$FRONTEND_CHAIN_JSON" ]] && (($(jq length <<<"$versions") > $(jq '.walletInitVersions | length' "$FRONTEND_CHAIN_JSON"))); then
    warn "Paymaster 或代币变了：追加钱包初始化参数第 $(jq '.[-1].version' <<<"$versions") 版（老钱包继续用旧版本，ADR 0008）"
  fi
  mkdir -p "$(dirname "$FRONTEND_CHAIN_JSON")" "$STATE_DIR"
  jq -n --argjson chainId "$CHAIN_ID" --arg name "$CHAIN_NAME" --arg rpc "$PUBLIC_RPC" --arg bundler "$PUBLIC_BUNDLER" \
    --arg ep "$ENTRY_POINT" --arg impl "$KERNEL_IMPL" --arg factory "$KERNEL_FACTORY" --arg meta "$KERNEL_META_FACTORY" \
    --arg webauthn "$WEBAUTHN_VALIDATOR" --arg v6492 "$ERC6492_VERIFIER" --arg usdt "$TOKEN_ADDR" --arg config "$CONFIG" \
    --arg escrow "$ESCROW" --arg stream "$STREAM" --arg paymaster "$PAYMASTER" --arg signer "$PAYMASTER_SIGNER" \
    --arg deployer "$DEPLOYED_BY" --argjson versions "$versions" '{
      "//": "Arbitrum Sepolia 测试网配置，由 deploy/testnet/testnet.sh emit-config 生成，勿手改；walletInitVersions 只增不改（ADR 0008）。",
      chainId: $chainId,
      name: $name,
      rpcUrl: $rpc,
      bundlerUrl: $bundler,
      entryPoint: $ep,
      kernel: {version: "0.3.3", implementation: $impl, factory: $factory, metaFactory: $meta},
      webAuthnValidator: $webauthn,
      erc6492Verifier: $v6492,
      contracts: {MockUSDT: $usdt, EscrowConfig: $config, Escrow: $escrow, StreamEscrow: $stream, FixedFeePaymaster: $paymaster},
      paymasterSigner: $signer,
      walletInitVersions: $versions,
      deployer: $deployer
    }' >"$FRONTEND_CHAIN_JSON"

  {
    cat <<TOML
# 由 deploy/testnet/testnet.sh emit-config 生成，勿手改。Arbitrum Sepolia 测试网（格式见 backend/crates/chain/chain.example.toml）。
# RPC 地址带密钥，不写在这里：服务器上由 CHAIN_RPC_PRIMARY_HTTP / CHAIN_RPC_PRIMARY_WS / CHAIN_RPC_BACKUP_HTTP /
# CHAIN_RPC_BACKUP_WS 提供（/etc/akari/*.env，见 deploy/README.md）。
chain_id = $CHAIN_ID
name = "$CHAIN_NAME"

[contracts]
escrow = "$ESCROW"
stream = "$STREAM"
config = "$CONFIG"
# 第一笔部署交易所在区块：indexer 首次启动从这里补拉。
start_block = $START_BLOCK

[token]
symbol = "$(cast call "$TOKEN_ADDR" 'symbol()(string)' | tr -d '"')"
address = "$TOKEN_ADDR"
decimals = 6

[aa]
entry_point = "$ENTRY_POINT"
paymaster = "$PAYMASTER"
factories = ["$KERNEL_META_FACTORY"]

# indexer 每轮补拉都要查区块与日志，是 RPC 用量的大头（docs/runbooks/testnet-deploy.md「RPC 用量」）。
[indexer]
reconcile_interval_ms = $INDEXER_RECONCILE_MS
TOML
    jq -r '.[] | "\n[[wallet_init_versions]]\nversion = \(.version)\nkernel_implementation = \"\(.kernelImpl)\"\nkernel_factory = \"\(.factory)\"\nkernel_meta_factory = \"\(.metaFactory)\"\nwebauthn_validator = \"\(.webauthnValidator)\"\nentry_point = \"\(.entryPoint)\"\npaymaster = \"\(.paymaster)\"\napprove_token = \"\(.approveToken)\"\napprove_amount = \"\(.approveAmount)\""' <<<"$versions"
  } >"$BACKEND_CHAIN_TOML"
  log "已写入 ${FRONTEND_CHAIN_JSON#"$ROOT/"}（前端）与 ${BACKEND_CHAIN_TOML#"$ROOT/"}（后端 CHAIN_CONFIG）"
}

# ───────────────────────────── 入口 ─────────────────────────────

main() {
  local cmd=${1:-}
  [[ "$cmd" =~ ^(preflight|deploy|configure|verify|emit-config)$ ]] ||
    die "用法：$0 preflight | deploy [--resume] | configure | verify | emit-config"
  shift
  load_config
  load_rpc
  # 有密钥时整个运行经过遮盖重跑一遍（forge / cast 的错误信息里可能带出 RPC URL）。
  if [[ -n "$REDACT" && -z "${AKARI_REDACTING:-}" ]]; then
    set +e
    trap - ERR # 出错位置由子进程报告
    AKARI_REDACTING=1 "$BASH" "${BASH_SOURCE[0]}" "$cmd" "$@" 2>&1 | redact
    exit "${PIPESTATUS[0]}"
  fi
  load_wallet
  case "$cmd" in
    preflight) preflight ;;
    deploy) deploy "$@" ;;
    configure) configure ;;
    verify) verify ;;
    emit-config) emit_config ;;
  esac
}

main "$@"
