#!/usr/bin/env bash
# 测试网部署演练：本机 anvil fork Arbitrum Sepolia（链 ID 保持 421614），用 testnet.sh 走一遍真实流程。
#   deploy/testnet/rehearse.sh up        计数转发（rpc-meter.py）→ anvil fork → 建 3 个 Safe（链上现成的 Safe v1.4.1 工厂）、
#                                        一次性部署密钥库与 forum-sponsor 热密钥 → 写演练用的 testnet.sh 配置
#   deploy/testnet/rehearse.sh run       testnet.sh preflight → deploy → configure → 冒充 Safe 执行 Transaction Builder 文件 →
#                                        configure（应无事可做）→ verify → emit-config
#   deploy/testnet/rehearse.sh services  独立 PostgreSQL 容器 + forum-api / worker / indexer / sponsor（读 emit-config 的链配置）
#   deploy/testnet/rehearse.sh smoke     钱包地址核对 + forum-sponsor 模拟并签一笔付款操作
#   deploy/testnet/rehearse.sh down      停掉本脚本启动的进程与容器，打印上游 RPC 各方法的请求数
# fork 来源：RPC_SECRETS_FILE 里的 ARBITRUM_SEPOLIA_RPC_URL（经计数转发，密钥不进 anvil 的命令行），没有时用公共 RPC。
# 运行状态全部在 <检出>/.run/rehearsal/（不入库）；只停自己记录了 PID 的进程。
# 演练里的私钥都是这次随机生成、只存在 .run 里的一次性密钥，fork 结束即作废；绝不能用在真实网络。
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
LOG_TAG=rehearse
# shellcheck source-path=SCRIPTDIR source=../lib.sh
source "$ROOT/deploy/lib.sh"

RUN="$ROOT/.run/rehearsal"
ANVIL_PORT=${REHEARSAL_ANVIL_PORT:-8345}
METER_PORT=${REHEARSAL_METER_PORT:-8346}
SERVICE_METER_PORT=${REHEARSAL_SERVICE_METER_PORT:-8347}
API_PORT=${REHEARSAL_API_PORT:-8480}
SPONSOR_PORT=${REHEARSAL_SPONSOR_PORT:-8490}
PG_PORT=${REHEARSAL_PG_PORT:-5604}
PG_CONTAINER=${REHEARSAL_PG_CONTAINER:-akari-pg-testnet}
VALKEY_URL=${REHEARSAL_VALKEY_URL:-redis://127.0.0.1:6379/14}
PUBLIC_RPC=https://sepolia-rollup.arbitrum.io/rpc
RPC="http://127.0.0.1:$ANVIL_PORT"
DATABASE_URL="postgres://akari:akari-rehearsal-only@127.0.0.1:$PG_PORT/akari"
TESTNET_ENV="$RUN/testnet.env"
STATE="$RUN/state"

# Safe v1.4.1（Arbitrum Sepolia 上的官方部署，up 时核对有代码）。
SAFE_FACTORY=0x4e1DCf7AD4e460CfD30791CCC4F9c8a4f820ec67
SAFE_SINGLETON_L2=0x29fcB43b46531BcA003ddC8FCB67FFE91900C762
SAFE_FALLBACK_HANDLER=0xfd0732Dc9E303f09fCEf3a7388Ad10A83459Ec99
ANVIL_FUNDER=0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266 # anvil 默认账户 0（anvil 自动解锁，fork 上有 10000 ETH）

export PATH="$HOME/.foundry/bin:$HOME/.cargo/bin:$PATH"
export NO_PROXY="${NO_PROXY:+$NO_PROXY,}127.0.0.1,localhost,::1,127.0.0.0/8" no_proxy="${no_proxy:+$no_proxy,}127.0.0.1,localhost,::1,127.0.0.0/8"
export ETH_RPC_URL=$RPC

pid_alive() { [[ -f "$RUN/$1.pid" ]] && kill -0 "$(<"$RUN/$1.pid")" 2>/dev/null; }

start_bg() { # start_bg <名字> <命令…>：后台启动，PID 记在 $RUN/<名字>.pid，日志 $RUN/logs/<名字>.log
  local name=$1; shift
  if pid_alive "$name"; then log "$name 已在运行（PID $(<"$RUN/$name.pid")）"; return 0; fi
  mkdir -p "$RUN/logs"
  "$@" >"$RUN/logs/$name.log" 2>&1 &
  echo $! >"$RUN/$name.pid"
}

stop_bg() { # 只停自己记录的 PID，并核对它仍是当初启动的程序
  local name=$1 pid
  [[ -f "$RUN/$name.pid" ]] || return 0
  pid=$(<"$RUN/$name.pid")
  if kill -0 "$pid" 2>/dev/null && tr '\0' ' ' <"/proc/$pid/cmdline" | grep -q -- "$2"; then
    kill "$pid"
    log "已停止 $name（PID $pid）"
  fi
  rm -f "$RUN/$name.pid"
}

meter_up() { ss -ltnH "sport = :$1" | grep -q .; } # 只看端口在听：不发请求，不占上游额度

label_address() { cast keccak "akari-rehearsal:$1" | cut -c1-42 | xargs cast to-check-sum-address; }

new_key() { # new_key <名字>：随机 secp256k1 私钥写进 $RUN/keys/<名字>（0600，地址在 <名字>.address），打印地址
  local f="$RUN/keys/$1" json
  if [[ ! -s "$f" ]]; then
    json=$(cast wallet new --json | jq '(.data // .) | if type == "array" then .[0] else . end')
    (umask 077 && jq -r .private_key <<<"$json" >"$f")
    jq -r .address <<<"$json" >"$f.address"
  fi
  cat "$f.address"
}

create_safe() { # create_safe <名字> <门槛> <签名人数>：经 Safe v1.4.1 工厂建 Safe，打印 "地址 签名人列表"
  local name=$1 threshold=$2 n=$3 owners=() i init safe
  for ((i = 1; i <= n; i++)); do owners+=("$(label_address "$name-owner-$i")"); done
  init=$(cast calldata 'setup(address[],uint256,address,bytes,address,address,uint256,address)' \
    "[$(IFS=,; echo "${owners[*]}")]" "$threshold" 0x0000000000000000000000000000000000000000 0x \
    "$SAFE_FALLBACK_HANDLER" 0x0000000000000000000000000000000000000000 0 0x0000000000000000000000000000000000000000)
  safe=$(cast call --from "$ANVIL_FUNDER" "$SAFE_FACTORY" 'createProxyWithNonce(address,bytes,uint256)(address)' \
    "$SAFE_SINGLETON_L2" "$init" 0)
  if ! has_code "$safe"; then # 再次 up（anvil 仍在运行）时 Safe 已经建好
    cast send --unlocked --from "$ANVIL_FUNDER" "$SAFE_FACTORY" 'createProxyWithNonce(address,bytes,uint256)' \
      "$SAFE_SINGLETON_L2" "$init" 0 >/dev/null
  fi
  echo "$safe $(IFS=,; echo "${owners[*]}")"
}

up() {
  mkdir -p "$RUN/keys" "$STATE"
  chmod 700 "$RUN/keys"
  local upstream=$PUBLIC_RPC
  if [[ -n "${RPC_SECRETS_FILE:-}" ]]; then
    private_file "$RPC_SECRETS_FILE" RPC_SECRETS_FILE
    upstream=$(secret_value "$RPC_SECRETS_FILE" ARBITRUM_SEPOLIA_RPC_URL)
    [[ "$upstream" == https://* ]] || die "RPC_SECRETS_FILE 缺 ARBITRUM_SEPOLIA_RPC_URL"
    log "fork 来源：RPC_SECRETS_FILE 主节点（经计数转发 127.0.0.1:$METER_PORT）"
  else
    log "fork 来源：$PUBLIC_RPC（经计数转发 127.0.0.1:$METER_PORT）"
  fi
  RPC_METER_UPSTREAM=$upstream start_bg rpc-meter python3 "$HERE/rpc-meter.py" "$METER_PORT" "$RUN/rpc-counts.json"
  unset upstream
  wait_for "计数转发" meter_up "$METER_PORT"
  # Osaka 带 EIP-7951 secp256r1 预编译（0x100，与 Arbitrum 的 RIP-7212 相同）；链 ID 沿用 fork 来源的 421614。
  start_bg anvil anvil --host 127.0.0.1 --port "$ANVIL_PORT" --fork-url "http://127.0.0.1:$METER_PORT" \
    --hardfork osaka --block-time 1 ${REHEARSAL_FORK_BLOCK:+--fork-block-number "$REHEARSAL_FORK_BLOCK"}
  wait_for "anvil（$RPC）" cast chain-id
  [[ "$(cast chain-id)" == 421614 ]] || die "fork 的链 ID 不是 421614"
  # anvil 没有 Arbitrum 的 NodeInterface（0xC8，虚拟合约）：放一个把 L1 数据费估成 0 的替身，forum-sponsor 估 gas 时调用它。
  cast rpc anvil_setCode 0x00000000000000000000000000000000000000C8 0x60606000f3 >/dev/null

  log "建 Safe（v1.4.1 工厂 $SAFE_FACTORY）"
  local c
  for c in "$SAFE_FACTORY" "$SAFE_SINGLETON_L2" "$SAFE_FALLBACK_HANDLER"; do check_code Safe-v1.4.1 "$c"; done
  local arbiter treasury ops arbiter_owners treasury_owners ops_owners
  read -r arbiter arbiter_owners < <(create_safe arbiter 2 3)
  read -r treasury treasury_owners < <(create_safe treasury 2 3)
  read -r ops ops_owners < <(create_safe ops 1 2)
  log "  仲裁 $arbiter · 金库 $treasury · 运营 $ops"

  log "一次性密钥（只在 $RUN/keys）"
  local password="$RUN/keys/deployer.password" keystore="$RUN/keys/deployer.json" deployer
  if [[ ! -f "$keystore" ]]; then
    (umask 077 && openssl rand -hex 24 >"$password")
    (umask 077 && CAST_PASSWORD=$(<"$password") cast wallet new "$RUN/keys" deployer.json >/dev/null)
  fi
  deployer=$(cast wallet address --keystore "$keystore" --password-file "$password")
  # 像从水龙头领一样只给 0.2 ETH，preflight 的余额检查才有意义。
  cast rpc anvil_setBalance "$deployer" "$(cast to-hex "$(cast to-wei 0.2)")" >/dev/null
  cast rpc anvil_setBalance "$ops" "$(cast to-hex "$(cast to-wei 0.05)")" >/dev/null

  cat >"$TESTNET_ENV" <<ENV
# 由 deploy/testnet/rehearse.sh up 生成：本机 fork 演练，地址与密钥都是一次性的。
RPC_URL=$RPC
DEPLOYER_KEYSTORE=$keystore
DEPLOYER_PASSWORD_FILE=$password
DEPLOYER_ADDRESS=$deployer
ARBITER_SAFE=$arbiter
ARBITER_SAFE_OWNERS=$arbiter_owners
ARBITER_SAFE_THRESHOLD=2
TREASURY_SAFE=$treasury
TREASURY_SAFE_OWNERS=$treasury_owners
TREASURY_SAFE_THRESHOLD=2
OPS_SAFE=$ops
OPS_SAFE_OWNERS=$ops_owners
OPS_SAFE_THRESHOLD=1
PAYMASTER_SIGNER=$(new_key paymaster-signer)
RELAYER_ADDRESS=$(new_key relayer-hot-wallet)
AUCTION_SIGNER=$(new_key auction-signer)
TOKEN=
STATE_DIR=$STATE
FRONTEND_CHAIN_JSON=$STATE/chain.arbitrum-sepolia.json
ENV
  log "就绪：anvil $RPC（fork 区块 $(cast rpc anvil_nodeInfo | jq -r .forkConfig.forkBlockNumber)）· 配置 ${TESTNET_ENV#"$ROOT/"}"
}

testnet() { TESTNET_ENV=$TESTNET_ENV "$HERE/testnet.sh" "$@"; }

# 冒充 Safe 执行 configure 生成的 Transaction Builder 文件（真实测试网上由签名人在 Safe 界面执行）。
exec_safe_batches() {
  local f safe n i
  for f in "$STATE"/safe-tx/*.json; do
    [[ -f "$f" ]] || continue
    safe=$(jq -r .meta.createdFromSafeAddress "$f")
    n=$(jq '.transactions | length' "$f")
    log "冒充 Safe $safe 执行 ${f#"$ROOT/"}（$n 笔）"
    cast rpc anvil_impersonateAccount "$safe" >/dev/null
    for ((i = 0; i < n; i++)); do
      cast send --unlocked --from "$safe" "$(jq -r ".transactions[$i].to" "$f")" "$(jq -r ".transactions[$i].data" "$f")" \
        --value "$(jq -r ".transactions[$i].value" "$f")" >/dev/null
    done
    cast rpc anvil_stopImpersonatingAccount "$safe" >/dev/null
  done
}

run() {
  [[ -f "$TESTNET_ENV" ]] || die "先运行 $0 up"
  testnet deploy # 先跑 preflight
  testnet configure
  exec_safe_batches
  log "再跑一次 configure：链上应已与配置一致，不再生成 Safe 文件"
  testnet configure
  compgen -G "$STATE/safe-tx/*.json" >/dev/null && die "第二次 configure 仍有待执行的 Safe 交易"
  testnet verify
  testnet emit-config
  # 演练链的 gas 价与真实测试网不同（anvil 不算 L1 数据费），只看 L2 gas 用量。
  local g total=0 n=0
  while read -r g; do total=$((total + g)); n=$((n + 1)); done < <(jq -r '.receipts[].gasUsed' \
    "$STATE/broadcast/Deploy.s.sol/421614/run-latest.json")
  log "部署交易 $n 笔，L2 gas 合计 $total"
}

service_env() { # 四个服务共用的环境变量（在子 shell 里调用）
  export DATABASE_URL VALKEY_URL KV_PREFIX=testnet AKARI_ENV=development RUST_LOG=info \
    CHAIN_CONFIG="$STATE/chain.toml" CHAIN_RPC_PRIMARY_HTTP="http://127.0.0.1:$SERVICE_METER_PORT" \
    CHAIN_RPC_PRIMARY_WS="ws://127.0.0.1:$ANVIL_PORT" \
    SPONSOR_URL="http://127.0.0.1:$SPONSOR_PORT"
  SPONSOR_API_TOKEN=$(<"$RUN/keys/sponsor-api-token")
  export SPONSOR_API_TOKEN
}

services() {
  [[ -f "$STATE/chain.toml" ]] || die "先运行 $0 run"
  if ! docker inspect "$PG_CONTAINER" >/dev/null 2>&1; then
    log "PostgreSQL 容器 $PG_CONTAINER（127.0.0.1:$PG_PORT，内存上限 2 GB）"
    docker run -d --name "$PG_CONTAINER" --memory 2g -p "127.0.0.1:$PG_PORT:5432" \
      -e POSTGRES_USER=akari -e POSTGRES_PASSWORD=akari-rehearsal-only -e POSTGRES_DB=akari \
      groonga/pgroonga:4.1.0-debian-18 >/dev/null
  fi
  # 经 TCP 检查：镜像初始化期间的临时服务器只听 Unix socket，TCP 通了才是正式启动。
  wait_for PostgreSQL docker exec "$PG_CONTAINER" pg_isready -h 127.0.0.1 -U akari -d akari
  (cd "$ROOT/backend" && sqlx migrate run --source crates/db/migrations --database-url "$DATABASE_URL")
  local bin="$ROOT/backend/target/debug"
  (cd "$ROOT/backend" && CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-6} cargo build -q -p forum-api -p forum-worker -p forum-indexer -p forum-sponsor)
  # 四个程序的 HTTP RPC 也经计数转发（上游是 anvil），估算真实节点上的用量；WebSocket 订阅直连 anvil。
  RPC_METER_UPSTREAM=$RPC start_bg service-meter python3 "$HERE/rpc-meter.py" "$SERVICE_METER_PORT" "$RUN/service-rpc-counts.json"
  wait_for "计数转发（服务）" meter_up "$SERVICE_METER_PORT"
  [[ -f "$RUN/services.started" ]] || date +%s >"$RUN/services.started"
  local arbiter
  arbiter=$(sed -n 's/^ARBITER_SAFE=//p' "$TESTNET_ENV")
  [[ -f "$RUN/keys/sponsor-api-token" ]] || (umask 077 && openssl rand -hex 32 >"$RUN/keys/sponsor-api-token")
  # 密钥与令牌只经环境变量（子 shell 里 export 后启动），不出现在任何进程的命令行参数里。
  (
    service_env
    # 演练链 ID 是 421614（不是主网），forum-sponsor 接受环境变量里的一次性私钥；真实服务器上走 systemd 凭据。
    SPONSOR_LISTEN="127.0.0.1:$SPONSOR_PORT"
    SPONSOR_DEV_PAYMASTER_KEY=$(<"$RUN/keys/paymaster-signer")
    SPONSOR_DEV_HOT_WALLET_KEY=$(<"$RUN/keys/relayer-hot-wallet")
    SPONSOR_DEV_AUCTION_KEY=$(<"$RUN/keys/auction-signer")
    export SPONSOR_LISTEN SPONSOR_DEV_PAYMASTER_KEY SPONSOR_DEV_HOT_WALLET_KEY SPONSOR_DEV_AUCTION_KEY
    start_bg forum-sponsor "$bin/forum-sponsor"
  )
  (service_env && start_bg forum-indexer "$bin/forum-indexer")
  (
    service_env
    export SMTP_URL=smtp://127.0.0.1:1 MAIL_FROM=rehearsal@localhost # 演练不发邮件
    start_bg forum-worker "$bin/forum-worker"
  )
  (
    service_env
    COOKIE_SECRET=$(openssl rand -hex 64)
    AUTH_SECRET=$(openssl rand -hex 32)
    export API_LISTEN="127.0.0.1:$API_PORT" COOKIE_SECRET AUTH_SECRET COOKIE_SECURE=false BRAND_NAME=Akari \
      ALLOWED_ORIGINS=http://localhost:3000 WALLET_RP_ID=localhost WALLET_ORIGINS=http://localhost:3001 \
      ADMIN_RP_ID=localhost ADMIN_ORIGIN=http://localhost:3002 RULING_RP_ID=localhost RULING_ORIGIN=http://localhost:3000 \
      ARBITRATION_SAFE="$arbiter"
    start_bg forum-api "$bin/forum-api"
  )
  wait_for forum-sponsor curl -sf "http://127.0.0.1:$SPONSOR_PORT/healthz"
  wait_for forum-api curl -sf "http://127.0.0.1:$API_PORT/api/v1/trade/params"
  sleep 5
  local name
  for name in forum-sponsor forum-indexer forum-worker forum-api; do
    pid_alive "$name" || die "$name 退出了，日志 $RUN/logs/$name.log"
  done
  log "四个服务都在运行：forum-api 127.0.0.1:$API_PORT · forum-sponsor 127.0.0.1:$SPONSOR_PORT（日志 ${RUN#"$ROOT/"}/logs）"
}

smoke() {
  pid_alive forum-sponsor && pid_alive forum-api || die "先运行 $0 services"
  local json="$STATE/chain.arbitrum-sepolia.json" api="http://127.0.0.1:$API_PORT" wallet user token
  log "forum-api 的钱包初始化参数与前端链配置一致（GET /api/v1/trade/params）"
  diff <(curl -sf "$api/api/v1/trade/params" | jq -S '[.wallet_init_versions[] | map_values(tostring | ascii_downcase)]') \
    <(jq -S '[.walletInitVersions[] | {version, kernel_implementation: .kernelImpl, kernel_factory: .factory,
        kernel_meta_factory: .metaFactory, webauthn_validator: .webauthnValidator, entry_point: .entryPoint,
        paymaster, approve_token: .approveToken, approve_amount: .approveAmount} | map_values(tostring | ascii_downcase)]' "$json") >&2 ||
    die "forum-api 与前端链配置的钱包初始化参数不一致"
  log "  ✓ 一致"

  log "钱包地址：钱包 SDK 推算 = 链上 Kernel 工厂（chain-smoke.test.ts）"
  (cd "$ROOT/frontend/apps/wallet" && CHAIN_SMOKE_JSON=$json CHAIN_SMOKE_RPC=$RPC CHAIN_SMOKE_OUT="$RUN/smoke-wallet" \
    pnpm exec vitest run lib/wallet/chain-smoke.test.ts -t 钱包地址)
  wallet=$(<"$RUN/smoke-wallet")
  log "  ✓ 钱包 $wallet"

  log "登记测试用户与钱包（直接写库），给钱包 mint 20 mUSDT"
  user=$(docker exec -i "$PG_CONTAINER" psql -U akari -d akari -v ON_ERROR_STOP=1 -qtA <<SQL
INSERT INTO users (email) VALUES ('smoke@rehearsal.invalid') ON CONFLICT (email) DO NOTHING;
-- 公钥列只为满足约束（forum-sponsor 只核对钱包地址；地址由测试里的固定公钥推算）。
INSERT INTO passkeys (user_id, purpose, credential_id, credential, public_key_cose, pubkey_x, pubkey_y)
  SELECT id, 1, '\\x736d6f6b65', '{}'::jsonb, '\\x00', decode(repeat('11', 32), 'hex'), decode(repeat('22', 32), 'hex')
  FROM users WHERE email = 'smoke@rehearsal.invalid'
  ON CONFLICT (credential_id) DO NOTHING;
INSERT INTO wallets (user_id, chain_id, address, passkey_id, init_version)
  SELECT u.id, 421614, '\\x${wallet#0x}', p.id, 1 FROM users u JOIN passkeys p ON p.user_id = u.id
  WHERE u.email = 'smoke@rehearsal.invalid' ON CONFLICT (user_id, chain_id) DO NOTHING;
SELECT id FROM users WHERE email = 'smoke@rehearsal.invalid';
SQL
  )
  cast send --unlocked --from "$ANVIL_FUNDER" "$(jq -r .contracts.MockUSDT "$json")" 'mint(address,uint256)' \
    "$wallet" 20000000 >/dev/null

  log "forum-sponsor 模拟并签一笔付款（approve + createAndFund，钱包未部署：initCode 经 meta factory）"
  new_key seller >/dev/null
  token=$(<"$RUN/keys/sponsor-api-token")
  local before
  before=$(jq -c .methods "$RUN/service-rpc-counts.json")
  (cd "$ROOT/frontend/apps/wallet" && CHAIN_SMOKE_JSON=$json CHAIN_SMOKE_RPC=$RPC \
    CHAIN_SMOKE_SPONSOR_URL="http://127.0.0.1:$SPONSOR_PORT" CHAIN_SMOKE_SPONSOR_TOKEN=$token CHAIN_SMOKE_USER_ID=$user \
    CHAIN_SMOKE_SELLER_KEY=$(<"$RUN/keys/seller") pnpm exec vitest run lib/wallet/chain-smoke.test.ts -t forum-sponsor)
  log "  ✓ 签名通过；forum-sponsor 模拟方法计数：$(curl -sf "http://127.0.0.1:$SPONSOR_PORT/healthz" | jq -c .simulations)"
  log "  这笔操作期间四个程序的 HTTP RPC 请求：$(jq -c --argjson b "$before" '.methods | with_entries(.value -= ($b[.key] // 0))
    | with_entries(select(.value > 0))' "$RUN/service-rpc-counts.json")"
  log "四个程序运行 $(($(date +%s) - $(<"$RUN/services.started"))) 秒的 HTTP RPC 请求：$(jq -c . "$RUN/service-rpc-counts.json")"
}

down() {
  local name
  for name in forum-api forum-worker forum-indexer forum-sponsor; do stop_bg "$name" "$name"; done
  stop_bg service-meter rpc-meter.py
  rm -f "$RUN/services.started"
  stop_bg anvil anvil
  stop_bg rpc-meter rpc-meter.py
  if docker inspect "$PG_CONTAINER" >/dev/null 2>&1; then
    docker rm -f "$PG_CONTAINER" >/dev/null
    log "已删除容器 $PG_CONTAINER"
  fi
  if [[ -f "$RUN/rpc-counts.json" ]]; then
    log "上游 RPC 请求（按方法）：$(jq -c . "$RUN/rpc-counts.json")"
  fi
}

case "${1:-}" in
  up) up ;;
  run) run ;;
  services) services ;;
  smoke) smoke ;;
  down) down ;;
  *) die "用法：$0 up | run | services | smoke | down" ;;
esac
