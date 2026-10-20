#!/usr/bin/env bash
# sslip 测试服务器（docs/runbooks/staging.md）的 /etc/akari/*.env 与 SeaweedFS 身份文件。只用于 staging：
# 服务器端密钥（COOKIE_SECRET、AUTH_SECRET、SPONSOR_API_TOKEN、S3 访问密钥）在这里生成，不离开服务器；
# 带密钥的 RPC 地址只经 stdin 传入。可重复执行：已有的密钥保留，其余项按参数重写；手工加的其他行不动。
# 由 Ansible（staging_local 角色）安装为 /usr/local/sbin/akari-staging-env，root 运行：
#   sed -n 2p ~/secrets/infura-rpc.txt | ssh root@<服务器> akari-staging-env init <论坛主机> <钱包主机> <后台主机> <图片主机>
set -euo pipefail
umask 077

ETC=/etc/akari
BUCKET=akari-media
SPONSOR_LISTEN=127.0.0.1:8090
BACKUP_HTTP=https://sepolia-rollup.arbitrum.io/rpc

die() { echo "akari-staging-env: $*" >&2; exit 1; }

[ "$(id -u)" = 0 ] || die "run as root"
grep -qx 'AKARI_ENV=staging' "$ETC/deployment.env" 2>/dev/null || die "$ETC/deployment.env is not AKARI_ENV=staging"

# get <文件> <键>：已有的值（没有时为空）
get() { [ -f "$1" ] && sed -n "s/^$2=//p" "$1" | tail -n1 || true; }

# set_kv <文件> <键> <值>：替换或追加一行，文件始终 root 0600；值不出现在命令行参数里（awk 从环境变量读）
set_kv() {
  local file=$1 tmp
  tmp=$(mktemp "$file.XXXXXX")
  [ -f "$file" ] || : >"$file"
  K=$2 V=$3 awk 'BEGIN { k = ENVIRON["K"]; v = ENVIRON["V"]; done = 0 }
    index($0, k "=") == 1 { if (!done) print k "=" v; done = 1; next }
    { print }
    END { if (!done) print k "=" v }' "$file" >"$tmp"
  chmod 600 "$tmp"
  mv -f "$tmp" "$file"
}

# keep <文件> <键> <生成命令…>：已有就保留，否则生成
keep() {
  local file=$1 key=$2 v
  shift 2
  v=$(get "$file" "$key")
  [ -n "$v" ] || v=$("$@")
  printf '%s' "$v"
}

host_ok() { [[ "$1" =~ ^[a-z0-9]([a-z0-9.-]*[a-z0-9])?$ ]] || die "bad host name: $1"; }

init() {
  [ $# = 4 ] || die "usage: init <forum host> <wallet host> <admin host> <media host> (RPC URL on stdin)"
  local forum=$1 wallet=$2 admin=$3 media=$4 h
  for h in "$@"; do host_ok "$h"; done

  local rpc ws
  IFS= read -r rpc || true
  rpc=${rpc%$'\r'}
  [[ "$rpc" =~ ^https://arbitrum-sepolia\.infura\.io/v3/[0-9a-fA-F]+$ ]] || die "stdin must be the Infura Arbitrum Sepolia HTTPS URL"
  ws="wss://arbitrum-sepolia.infura.io/ws/v3/${rpc##*/}"

  [ -f "$ETC/chain.toml" ] || die "$ETC/chain.toml missing (the Ansible staging_local role installs it)"

  local api=$ETC/forum-api.env worker=$ETC/forum-worker.env indexer=$ETC/forum-indexer.env sponsor=$ETC/forum-sponsor.env web=$ETC/web.env
  local cookie auth token s3_key s3_secret
  cookie=$(keep "$api" COOKIE_SECRET openssl rand -hex 64)
  auth=$(keep "$api" AUTH_SECRET openssl rand -hex 32)
  token=$(keep "$sponsor" SPONSOR_API_TOKEN openssl rand -hex 32)
  s3_key=$(keep "$api" MEDIA_S3_ACCESS_KEY openssl rand -hex 10)
  s3_secret=$(keep "$api" MEDIA_S3_SECRET_KEY openssl rand -hex 32)

  local f
  for f in "$api" "$worker" "$indexer" "$sponsor"; do
    set_kv "$f" CHAIN_CONFIG "$ETC/chain.toml"
    set_kv "$f" CHAIN_RPC_PRIMARY_HTTP "$rpc"
    set_kv "$f" CHAIN_RPC_BACKUP_HTTP "$BACKUP_HTTP"
  done
  set_kv "$indexer" CHAIN_RPC_PRIMARY_WS "$ws"
  set_kv "$indexer" INDEXER_RECONCILE_MS 20000

  set_kv "$api" DATABASE_URL "postgres://forum_api@%2Frun%2Fpostgresql/akari"
  set_kv "$worker" DATABASE_URL "postgres://forum_worker@%2Frun%2Fpostgresql/akari"
  set_kv "$indexer" DATABASE_URL "postgres://forum_indexer@%2Frun%2Fpostgresql/akari"
  set_kv "$sponsor" DATABASE_URL "postgres://forum_sponsor@%2Frun%2Fpostgresql/akari"
  for f in "$api" "$worker"; do
    set_kv "$f" VALKEY_URL "unix:///run/valkey/valkey.sock"
    set_kv "$f" SPONSOR_URL "http://$SPONSOR_LISTEN"
    set_kv "$f" SPONSOR_API_TOKEN "$token"
    set_kv "$f" MEDIA_S3_ENDPOINT "https://$media"
    set_kv "$f" MEDIA_S3_BUCKET "$BUCKET"
    set_kv "$f" MEDIA_S3_REGION us-east-1
    set_kv "$f" MEDIA_S3_ACCESS_KEY "$s3_key"
    set_kv "$f" MEDIA_S3_SECRET_KEY "$s3_secret"
    set_kv "$f" MEDIA_PUBLIC_BASE "https://$media/$BUCKET/"
  done
  # forum-worker 启动时建存储桶（已存在时不报错）
  set_kv "$worker" MEDIA_S3_CREATE_BUCKET true

  set_kv "$api" COOKIE_SECRET "$cookie"
  set_kv "$api" AUTH_SECRET "$auth"
  set_kv "$api" BRAND_NAME "Akari Staging"
  set_kv "$api" ALLOWED_ORIGINS "https://$forum"
  set_kv "$api" INTERNAL_HOSTS "$forum"
  set_kv "$api" WALLET_RP_ID "$wallet"
  set_kv "$api" WALLET_ORIGINS "https://$wallet"
  set_kv "$api" ADMIN_RP_ID "$admin"
  set_kv "$api" ADMIN_ORIGIN "https://$admin"
  set_kv "$api" RULING_RP_ID "$forum"
  set_kv "$api" RULING_ORIGIN "https://$forum"
  # 测试网仲裁 Safe（docs/runbooks/testnet-deploy.md「当前部署」）
  set_kv "$api" ARBITRATION_SAFE 0xd6DA0736C4efbD85c6A9a2c11f68Ecb6D58de813

  # 邮件只进本机 Mailpit（SMTP 只接受本机地址，见 crates/mail/src/smtp.rs）
  set_kv "$worker" MAIL_FROM "Akari Staging <noreply@$forum>"
  set_kv "$worker" MAIL_PRIMARY smtp
  set_kv "$worker" SMTP_URL "smtp://127.0.0.1:1025"

  set_kv "$sponsor" SPONSOR_LISTEN "$SPONSOR_LISTEN"
  set_kv "$sponsor" SPONSOR_API_TOKEN "$token"
  # 告警线 = deploy/testnet/deployments/arbitrum-sepolia/sponsor-alerts.env
  set_kv "$sponsor" SPONSOR_ALERT_DEPOSIT_WEI 10000000000000000
  set_kv "$sponsor" SPONSOR_ALERT_HOT_WALLET_WEI 5000000000000000

  # Next.js 只拿 COOKIE_SECRET 的签名那一半（前 32 字节 = 64 个十六进制字符）
  set_kv "$web" COOKIE_SIGNING_KEY "${cookie:0:64}"
  set_kv "$web" API_ORIGIN "http://127.0.0.1:8081"

  # SeaweedFS：匿名只读（Caddy 只放行不带签名的正式目录 i/ 读取），forum-api / forum-worker 一个读写身份
  local s3=$ETC/seaweedfs-s3.json tmp
  tmp=$(mktemp "$s3.XXXXXX")
  K=$s3_key S=$s3_secret awk 'BEGIN {
    printf "{\"identities\": [\n  {\"name\": \"anonymous\", \"actions\": [\"Read\"]},\n"
    printf "  {\"name\": \"akari\", \"credentials\": [{\"accessKey\": \"%s\", \"secretKey\": \"%s\"}],\n", ENVIRON["K"], ENVIRON["S"]
    printf "   \"actions\": [\"Admin\", \"Read\", \"Write\", \"List\", \"Tagging\"]}\n]}\n"
  }' >"$tmp"
  chown seaweedfs:seaweedfs "$tmp"
  chmod 600 "$tmp"
  mv -f "$tmp" "$s3"

  chown root:root "$api" "$worker" "$indexer" "$sponsor" "$web"
  chmod 600 "$api" "$worker" "$indexer" "$sponsor" "$web"
  echo "akari-staging-env: wrote $api $worker $indexer $sponsor $web $s3 (secrets not shown)"
}

case "${1:-}" in
  init) shift; init "$@" ;;
  *) die "usage: akari-staging-env init <forum host> <wallet host> <admin host> <media host> (RPC URL on stdin)" ;;
esac
