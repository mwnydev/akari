#!/usr/bin/env bash
# 测试网 Safe 小工具（docs/runbooks/testnet-deploy.md「建 Safe」「执行 Safe 交易文件」）：用 cast 代替 app.safe.global，
# 签名人是本机密钥库。只用于测试网：主网的 Safe 由各签名人在自己的设备上签（硬件钱包 / Passkey）。
#   deploy/testnet/safe.sh create <名字> <门槛> <签名人,…>   经 Safe v1.4.1 工厂建 Safe（部署账户付 gas）；地址由名字决定，
#                                                         已建好时只核对并打印地址
#   deploy/testnet/safe.sh fund <Safe> <ETH>              部署账户把 Safe 余额补到 <ETH>（已够则不发）
#   deploy/testnet/safe.sh exec <交易文件.json> <签名人密钥库…>
#                                                         执行 testnet.sh configure 生成的 Transaction Builder 文件：签名人按
#                                                         EIP-712 签 SafeTx，部署账户提交 execTransaction；多笔经 MultiSendCallOnly
# 配置与 testnet.sh 相同（TESTNET_ENV、RPC_SECRETS_FILE、部署账户、可选 RPC_METER_URL）；签名人密钥库的口令文件
# SAFE_SIGNER_PASSWORD_FILE（0600，所有签名人共用）。exec 的结果追加到 <STATE_DIR>/safe-executions.jsonl（入库）；
# 同一个文件（按内容 sha256）只执行一次，要再执行先重新运行 configure 生成新文件。
set -Eeuo pipefail

SAFE_HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# 复用 testnet.sh 的配置、RPC、遮盖与 send（被 source 时不执行它的入口）。
# shellcheck source-path=SCRIPTDIR source=testnet.sh
source "$SAFE_HERE/testnet.sh"
LOG_TAG=safe
trap 'printf "\033[1;31m[safe]\033[0m 第 %s 行出错：%s\n" "$LINENO" "$BASH_COMMAND" >&2' ERR

# Safe v1.4.1 官方部署（Arbitrum Sepolia 上 eth_getCode 核对过；与 rehearse.sh 相同）。
SAFE_FACTORY=0x4e1DCf7AD4e460CfD30791CCC4F9c8a4f820ec67
SAFE_SINGLETON_L2=0x29fcB43b46531BcA003ddC8FCB67FFE91900C762
SAFE_FALLBACK_HANDLER=0xfd0732Dc9E303f09fCEf3a7388Ad10A83459Ec99
SAFE_MULTISEND_CALL_ONLY=0x9641d764fc13c8B624c04430C7356C1C7C8102e2
ZERO=0x0000000000000000000000000000000000000000
SAFE_TX_TYPE='SafeTx(address to,uint256 value,bytes data,uint8 operation,uint256 safeTxGas,uint256 baseGas,uint256 gasPrice,address gasToken,address refundReceiver,uint256 nonce)'
EXEC_SIG='execTransaction(address,uint256,bytes,uint8,uint256,uint256,uint256,address,address,bytes)'

is_address() { [[ "$1" =~ ^0x[0-9a-fA-F]{40}$ ]]; }

# ───────────────────────────── create ─────────────────────────────

# CREATE2 地址（与 SafeProxyFactory.createProxyWithNonce 相同算法），不依赖链上状态，Safe 已建好时也能算。
predict_safe() { # predict_safe <initializer> <saltNonce>
  local salt code_hash
  salt=$(cast keccak "$(cast concat-hex "$(cast keccak "$1")" "$(cast to-uint256 "$2")")")
  code_hash=$(cast keccak "$(cast concat-hex "$(cast call "$SAFE_FACTORY" 'proxyCreationCode()(bytes)')" \
    "$(cast to-uint256 "$SAFE_SINGLETON_L2")")")
  cast to-check-sum-address "0x$(cast keccak "$(cast concat-hex 0xff "$SAFE_FACTORY" "$salt" "$code_hash")" | cut -c27-)"
}

create() {
  (($# == 3)) || die "用法：$0 create <名字> <门槛> <签名人,…>"
  local name=$1 threshold=$2 owners=() o init salt safe
  [[ "$name" =~ ^[a-z0-9-]+$ ]] || die "名字只用小写字母、数字与 -"
  IFS=, read -r -a owners <<<"$3"
  for o in "${owners[@]}"; do is_address "$o" || die "签名人地址格式错误：$o"; done
  if [[ ! "$threshold" =~ ^[1-9][0-9]*$ ]] || ((threshold > ${#owners[@]})); then die "门槛必须在 1..${#owners[@]}"; fi
  [[ ",${3,,}," != *",${DEPLOYER_ADDRESS,,},"* ]] || die "部署账户不能当签名人"
  local c
  for c in "$SAFE_FACTORY" "$SAFE_SINGLETON_L2" "$SAFE_FALLBACK_HANDLER"; do has_code "$c" || die "Safe v1.4.1 合约 $c 没有代码"; done
  init=$(cast calldata 'setup(address[],uint256,address,bytes,address,address,uint256,address)' \
    "[$3]" "$threshold" "$ZERO" 0x "$SAFE_FALLBACK_HANDLER" "$ZERO" 0 "$ZERO")
  # saltNonce 取自名字：签名人与门槛相同的两个 Safe（仲裁 / 金库）地址也不同，重跑得到同一个地址。
  salt=$(cast to-dec "$(cast keccak "akari-testnet-safe:$name")")
  safe=$(predict_safe "$init" "$salt")
  if has_code "$safe"; then
    ok "Safe $name $safe 已存在"
  else
    [[ "$(cast call --from "$DEPLOYER_ADDRESS" "$SAFE_FACTORY" 'createProxyWithNonce(address,bytes,uint256)(address)' \
      "$SAFE_SINGLETON_L2" "$init" "$salt")" == "$safe" ]] || die "工厂返回的地址与推算的 $safe 不同"
    send "建 Safe $name（$threshold/${#owners[@]}）→ $safe" "$SAFE_FACTORY" 'createProxyWithNonce(address,bytes,uint256)' \
      "$SAFE_SINGLETON_L2" "$init" "$salt"
  fi
  expect "Safe $name 门槛" "$(call "$safe" 'getThreshold()(uint256)')" "$threshold"
  expect "Safe $name 签名人" "$(norm_list "$(cast call "$safe" 'getOwners()(address[])')")" "$(norm_list "$3")"
  expect "Safe $name 版本" "$(cast call "$safe" 'VERSION()(string)' | tr -d '"')" 1.4.1
  finish "create $name"
  printf '%s\n' "$safe"
}

# ───────────────────────────── fund ─────────────────────────────

fund() {
  (($# == 2)) || die "用法：$0 fund <Safe> <ETH>"
  is_address "$1" || die "地址格式错误：$1"
  has_code "$1" || die "$1 没有代码（不是 Safe）"
  local have want
  have=$(cast balance "$1")
  want=$(to_wei "$2")
  if wei_lt "$have" "$want"; then
    send "$1 余额补到 $2 ETH" "$1" --value "$((want - have))"
  else
    ok "$1 余额 $(eth "$have") ETH ≥ $2"
  fi
}

# ───────────────────────────── exec ─────────────────────────────

# MultiSendCallOnly 的打包格式：每笔 uint8 operation(0) ‖ address to ‖ uint256 value ‖ uint256 长度 ‖ data。
multisend_data() { # multisend_data <交易文件>
  local packed=0x n i to value data
  n=$(jq '.transactions | length' "$1")
  for ((i = 0; i < n; i++)); do
    read -r to value data < <(jq -r --argjson i "$i" '.transactions[$i] | [.to, .value, .data] | @tsv' "$1")
    packed=$(cast concat-hex "$packed" 0x00 "$to" "$(cast to-uint256 "$value")" \
      "$(cast to-uint256 $(((${#data} - 2) / 2)))" "$data")
  done
  cast calldata 'multiSend(bytes)' "$packed"
}

# 本地按 EIP-712 算 SafeTx 哈希，与 Safe 合约的 getTransactionHash 对照（签之前确认签的就是这笔交易）。
local_safe_tx_hash() { # <safe> <to> <value> <data> <operation> <nonce>
  local domain struct
  domain=$(cast keccak "$(cast abi-encode 'f(bytes32,uint256,address)' \
    "$(cast keccak 'EIP712Domain(uint256 chainId,address verifyingContract)')" "$CHAIN_ID" "$1")")
  struct=$(cast keccak "$(cast abi-encode 'f(bytes32,address,uint256,bytes32,uint8,uint256,uint256,uint256,address,address,uint256)' \
    "$(cast keccak "$SAFE_TX_TYPE")" "$2" "$3" "$(cast keccak "$4")" "$5" 0 0 0 "$ZERO" "$ZERO" "$6")")
  cast keccak "$(cast concat-hex 0x1901 "$domain" "$struct")"
}

typed_data() { # <safe> <to> <value> <data> <operation> <nonce>：cast wallet sign --data 用的 EIP-712 JSON
  jq -cn --argjson chain "$CHAIN_ID" --arg safe "$1" --arg to "$2" --arg value "$3" --arg data "$4" \
    --argjson op "$5" --arg nonce "$6" --arg zero "$ZERO" '{
      types: {
        EIP712Domain: [{name: "chainId", type: "uint256"}, {name: "verifyingContract", type: "address"}],
        SafeTx: [{name: "to", type: "address"}, {name: "value", type: "uint256"}, {name: "data", type: "bytes"},
          {name: "operation", type: "uint8"}, {name: "safeTxGas", type: "uint256"}, {name: "baseGas", type: "uint256"},
          {name: "gasPrice", type: "uint256"}, {name: "gasToken", type: "address"},
          {name: "refundReceiver", type: "address"}, {name: "nonce", type: "uint256"}]
      },
      primaryType: "SafeTx",
      domain: {chainId: $chain, verifyingContract: $safe},
      message: {to: $to, value: $value, data: $data, operation: $op, safeTxGas: "0", baseGas: "0", gasPrice: "0",
        gasToken: $zero, refundReceiver: $zero, nonce: $nonce}
    }'
}

exec_batch() {
  (($# >= 2)) || die "用法：$0 exec <交易文件.json> <签名人密钥库…>"
  local file=$1; shift
  [[ -f "$file" ]] || die "没有交易文件 $file"
  local pw=${SAFE_SIGNER_PASSWORD_FILE:?SAFE_SIGNER_PASSWORD_FILE 未设置（签名人密钥库口令文件，0600）}
  private_file "$pw" SAFE_SIGNER_PASSWORD_FILE
  [[ "$(jq -r .chainId "$file")" == "$CHAIN_ID" ]] || die "$file 不是链 $CHAIN_ID 的交易文件"
  local safe n digest log_file="$STATE_DIR/safe-executions.jsonl"
  safe=$(cast to-check-sum-address "$(jq -r .meta.createdFromSafeAddress "$file")")
  n=$(jq '.transactions | length' "$file")
  ((n > 0)) || die "$file 里没有交易"
  digest=$(sha256sum "$file" | cut -d' ' -f1)
  if [[ -f "$log_file" ]] && jq -e --arg d "$digest" 'select(.sha256 == $d)' "$log_file" >/dev/null; then
    die "$file 已执行过（$log_file）；重新运行 testnet.sh configure，仍需要时它会生成新文件"
  fi
  log "Safe $safe 执行 ${file#"$ROOT/"}（$n 笔）：$(jq -r .meta.description "$file")"

  local to value data op total
  total=0
  for value in $(jq -r '.transactions[].value' "$file"); do
    [[ "$value" =~ ^[0-9]{1,18}$ ]] || die "交易金额 $value wei 不是不超过 18 位的整数"
    total=$((total + value))
  done
  if ((n == 1)); then
    read -r to value data < <(jq -r '.transactions[0] | [.to, .value, .data] | @tsv' "$file")
    op=0
  else
    to=$SAFE_MULTISEND_CALL_ONLY value=0 op=1 # delegatecall MultiSendCallOnly：每笔仍是 Safe 发出的普通 call
    data=$(multisend_data "$file")
  fi
  if [[ "$data" != 0x ]] && ! has_code "$to"; then die "目标 $to 没有代码（带 calldata 的交易应调用合约）"; fi
  if wei_lt "$(cast balance "$safe")" "$total"; then
    die "Safe $safe 余额不足 $(eth "$total")（先 $0 fund $safe <ETH>）"
  fi

  local nonce threshold owners hash
  nonce=$(call "$safe" 'nonce()(uint256)')
  threshold=$(call "$safe" 'getThreshold()(uint256)')
  owners=",$(norm_list "$(cast call "$safe" 'getOwners()(address[])')"),"
  hash=$(cast call "$safe" 'getTransactionHash(address,uint256,bytes,uint8,uint256,uint256,uint256,address,address,uint256)(bytes32)' \
    "$to" "$value" "$data" "$op" 0 0 0 "$ZERO" "$ZERO" "$nonce")
  [[ "$hash" == "$(local_safe_tx_hash "$safe" "$to" "$value" "$data" "$op" "$nonce")" ]] ||
    die "Safe 合约的 SafeTx 哈希与本地 EIP-712 计算不一致"
  log "  SafeTx nonce $nonce，哈希 $hash，门槛 $threshold"

  # 签名人：必须是 Safe 的签名人，去重后按地址升序取前 <门槛> 个（Safe 要求签名按签名人地址升序拼接）。
  local ks addr signers=() typed sigs=0x signed=()
  typed=$(typed_data "$safe" "$to" "$value" "$data" "$op" "$nonce")
  for ks in "$@"; do
    private_file "$ks" 签名人密钥库
    addr=$(cast wallet address --keystore "$ks" --password-file "$pw")
    [[ "$owners" == *",${addr,,},"* ]] || die "$addr（$ks）不是 Safe $safe 的签名人"
    signers+=("${addr,,} $ks")
  done
  mapfile -t signers < <(printf '%s\n' "${signers[@]}" | sort -u -k1,1)
  ((${#signers[@]} >= threshold)) || die "需要 $threshold 个不同的签名人，只给了 ${#signers[@]} 个"
  local entry
  for entry in "${signers[@]:0:threshold}"; do
    addr=${entry%% *} ks=${entry#* }
    sigs=$(cast concat-hex "$sigs" "$(cast wallet sign --keystore "$ks" --password-file "$pw" --data "$typed")")
    signed+=("$(cast to-check-sum-address "$addr")")
    ok "签名人 $(cast to-check-sum-address "$addr") 已签（EIP-712）"
  done
  # 链上核对签名（与 execTransaction 同一检查），再模拟执行，最后才发交易。
  cast call "$safe" 'checkSignatures(bytes32,bytes,bytes)' "$hash" "$data" "$sigs" >/dev/null || die "Safe.checkSignatures 未通过"
  [[ "$(cast call --from "$DEPLOYER_ADDRESS" "$safe" "$EXEC_SIG(bool)" "$to" "$value" "$data" "$op" 0 0 0 "$ZERO" "$ZERO" "$sigs")" == true ]] ||
    die "execTransaction 模拟失败"
  # safeTxGas = 0 时内层调用失败整笔回滚（GS013），所以交易成功即执行成功。
  send "execTransaction（Safe $safe nonce $nonce）" "$safe" "$EXEC_SIG" "$to" "$value" "$data" "$op" 0 0 0 "$ZERO" "$ZERO" "$sigs"
  jq -cn --arg file "${file#"$ROOT/"}" --arg sha "$digest" --arg safe "$safe" --argjson nonce "$nonce" --arg hash "$hash" \
    --arg tx "$LAST_TX" --argjson at "$(date +%s)" --arg signers "$(IFS=,; echo "${signed[*]}")" \
    --arg name "$(jq -r .meta.name "$file")" --arg desc "$(jq -r .meta.description "$file")" \
    --argjson txs "$(jq -c '[.transactions[] | {to, value, data}]' "$file")" \
    '{file: $file, sha256: $sha, name: $name, description: $desc, safe: $safe, nonce: $nonce, safeTxHash: $hash,
      signers: ($signers | split(",")), transactions: $txs, transactionHash: $tx, executedAt: $at}' >>"$log_file"
  log "已记入 ${log_file#"$ROOT/"}；下一步：testnet.sh configure"
}

safe_main() {
  local cmd=${1:-}
  [[ "$cmd" =~ ^(create|fund|exec)$ ]] || die "用法：$0 create <名字> <门槛> <签名人,…> | fund <Safe> <ETH> | exec <交易文件.json> <签名人密钥库…>"
  shift
  load_config
  load_rpc
  if [[ -n "$REDACT" && -z "${AKARI_REDACTING:-}" ]]; then # 与 testnet.sh 相同：整个运行经过遮盖重跑
    set +e
    trap - ERR
    AKARI_REDACTING=1 "$BASH" "${BASH_SOURCE[0]}" "$cmd" "$@" 2>&1 | redact
    exit "${PIPESTATUS[0]}"
  fi
  load_wallet
  [[ "$(cast chain-id)" == "$CHAIN_ID" ]] || die "RPC 链 ID 不是 $CHAIN_ID"
  case "$cmd" in
    create) create "$@" ;;
    fund) fund "$@" ;;
    exec) exec_batch "$@" ;;
  esac
}

safe_main "$@"
