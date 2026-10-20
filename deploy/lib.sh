# shellcheck shell=bash disable=SC2034 # 常量由 source 本文件的脚本使用
# 部署脚本共用（deploy/dev/aa.sh、deploy/testnet/*.sh source 本文件）：链上现成合约地址、日志、链上检查。
# RPC 一律经 ETH_RPC_URL 环境变量交给 cast（不放进命令行参数，带密钥的地址不会出现在进程列表里）。

# 链上现成合约（ADR 0003；2026-10-19 在 Arbitrum Sepolia 用 eth_getCode 核对过，表见 deploy/dev/README-aa.md）。
ENTRY_POINT=0x0000000071727De22E5E9d8BAf0edAc6f37da032
KERNEL_IMPL=0xd6CEDDe84be40893d153Be9d467CD6aD37875b28
KERNEL_FACTORY=0x2577507b78c2008Ff367261CB6285d44ba5eF2E9
KERNEL_META_FACTORY=0xd703aaE79538628d27099B8c4f621bE4CCd142d5
WEBAUTHN_VALIDATOR=0x7ab16Ff354AcB328452F1D445b3Ddee9a91e9e69 # gitleaks:allow 公开合约地址
ERC6492_VERIFIER=0x00007bd799e4A591FeA53f8A8a3E9f931626Ba7e
CREATE2_DEPLOYER=0x4e59b44847b379578588920cA78FbF26c0B4956C

LOG_TAG=${LOG_TAG:-deploy}
log() { printf '\033[1;34m[%s]\033[0m %s\n' "$LOG_TAG" "$*" >&2; }
die() { printf '\033[1;31m[%s]\033[0m %s\n' "$LOG_TAG" "$*" >&2; exit 1; }

wait_for() { # wait_for <描述> <命令…>
  local what=$1; shift
  for _ in $(seq 1 90); do
    if "$@" >/dev/null 2>&1; then return 0; fi
    sleep 1
  done
  die "$what 没有在 90 秒内就绪"
}

has_code() { [[ "$(cast code "$1")" != "0x" ]]; }

check_code() { # check_code <名字> <地址>：没有代码就退出
  has_code "$2" || die "$1 $2 没有代码"
  log "  ✓ $1 $2"
}

# P-256 预编译自检（地址 0x100）：RIP-7212 规范里的有效签名测试向量，返回 1 才算可用。
p256_ok() {
  [[ "$(cast call 0x0000000000000000000000000000000000000100 \
    --data 0x4cee90eb86eaa050036147a12d49004b6b9c72bd725d39d4785011fe190f0b4da73bd4903f0ce3b639bbbf6e8e80d16931ff4bcf5993d58468e8fb19086e8cac36dbcd03009df8c59286b162af3bd7fcc0450c9aa81be5d10d312af6c66b1d604aebd3099c618202fcfe16ae7770b0c49ab5eadf74b754204a3bb6060e44eff37618b065f9832de4ca6ca971a7a1adc826d0f7c00181a5fb2ddf79ae00b4e10e)" == *1 ]]
}

# 密钥文件里取一项（只读 KEY=VALUE，不 source，不执行文件内容）。
secret_value() {
  local line
  line=$(grep -E "^[[:space:]]*(export[[:space:]]+)?$2=" "$1" | tail -n1) || true
  line=${line#*=}
  line=${line%$'\r'}
  line=${line%\"}
  line=${line#\"}
  printf '%s' "$line"
}

private_file() { # 密钥 / 口令文件必须只有本人可读
  [[ -f "$1" ]] || die "$2 不存在：$1"
  [[ "$(stat -c %a "$1")" =~ ^[46]00$ ]] || die "$2 权限必须是 0600 或 0400：$1"
}
