#!/usr/bin/env bash
# sslip 测试服务器（docs/runbooks/staging.md）：本机构建一个版本并装上服务器。只用于 staging——生产只装 CI 构建、
# 带 GitHub 构建证明的产物；这里打出的包没有证明，install-release 只在服务器是 staging 时才接受（--unattested）。
#
#   deploy/staging/release.sh build <sslip 基础名>      如 203-0-113-7.sslip.io；产物在 .run/staging/
#   deploy/staging/release.sh install <服务器 IP>       上传最新产物并 sudo install-release install --unattested
#   deploy/staging/release.sh seed <服务器 IP>          板块（seeds/dev.sql）、打开拍卖；OWNER_EMAIL_FILE 里的邮箱已注册时授予站长（role 1）
#   deploy/staging/release.sh migrate <服务器 IP>       跑数据库迁移（本机 sqlx-cli 传到服务器，以 postgres 连接、
#                                                     SET ROLE akari_owner，表归属主所有；之后重跑 play 套用授权）
#
# 版本内容（与 roles/app、roles/caddy 的路径一致）：bin/forum-{api,worker,indexer,sponsor}（musl 静态）、
# forum/（Next.js standalone，server.js）、wallet/、admin/（静态导出，Caddy 直接提供）。
# 环境变量：SSH_KEY（默认 ~/.ssh/akari_staging）、KNOWN_HOSTS（默认 ~/.ssh/akari_staging_known_hosts）、
#           CARGO_BUILD_JOBS（默认 6）、RUST_IMAGE（默认 rust:1.99.1-trixie）、
#           SQLX_BIN（默认 PATH 里的 sqlx；须能在 Debian 13 x86_64 上运行，即在同样的系统上 cargo install 的）。
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
OUT=$ROOT/.run/staging
SSH_KEY=${SSH_KEY:-$HOME/.ssh/akari_staging}
KNOWN_HOSTS=${KNOWN_HOSTS:-$HOME/.ssh/akari_staging_known_hosts}
RUST_IMAGE=${RUST_IMAGE:-rust:1.99.1-trixie}
# Cloudflare 文档公开的 Turnstile 测试站点密钥（总是通过）；forum-api 在 production 拒绝对应的测试密钥
TURNSTILE_TEST_SITE_KEY=1x00000000000000000000AA

log() { printf '\033[1;34m[staging]\033[0m %s\n' "$*" >&2; }
die() { printf '\033[1;31m[staging]\033[0m %s\n' "$*" >&2; exit 1; }

build_backend() {
  log "backend: musl static build in $RUST_IMAGE (6 GB memory cap)"
  mkdir -p "$1/bin"
  docker run --rm --memory 6g --network host \
    -e HTTP_PROXY -e HTTPS_PROXY -e http_proxy -e https_proxy -e NO_PROXY -e no_proxy \
    -e CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-6}" -e SQLX_OFFLINE=true -e CARGO_TARGET_DIR=/target \
    -e OUT_UID="$(id -u)" -e OUT_GID="$(id -g)" \
    -v akari-cargo-reg:/usr/local/cargo/registry -v akari-musl-target:/target \
    -v "$ROOT/backend:/src:ro" -v "$ROOT/rust-toolchain.toml:/rust-toolchain.toml:ro" -v "$1/bin:/out" \
    -w /src "$RUST_IMAGE" bash -euc '
      DEBIAN_FRONTEND=noninteractive apt-get install -y -qq musl-tools cmake file >/dev/null 2>&1 ||
        { apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq musl-tools cmake file >/dev/null; }
      rustup target add x86_64-unknown-linux-musl >/dev/null 2>&1
      cargo build --release --locked --target x86_64-unknown-linux-musl --bins
      for b in forum-api forum-worker forum-indexer forum-sponsor; do
        file "/target/x86_64-unknown-linux-musl/release/$b" | grep -q "static-pie linked" || { echo "$b is not static" >&2; exit 1; }
        install -m 755 -o "$OUT_UID" -g "$OUT_GID" "/target/x86_64-unknown-linux-musl/release/$b" /out/
      done' || die "backend build failed"
}

build_frontend() {
  local base=$1 dest=$2 fe=$ROOT/frontend
  local forum=https://forum.$base wallet=wallet.$base admin=admin.$base
  log "frontend: forum (standalone), wallet + admin (static export) for $forum"
  (cd "$fe" && pnpm install --frozen-lockfile >/dev/null)
  (cd "$fe/apps/forum" &&
    NEXT_PUBLIC_WALLET_ORIGIN=https://$wallet NEXT_PUBLIC_TURNSTILE_SITE_KEY=$TURNSTILE_TEST_SITE_KEY \
      API_ORIGIN=http://127.0.0.1:8081 pnpm build)
  (cd "$fe/apps/wallet" &&
    NEXT_PUBLIC_WALLET_RP_ID=$wallet NEXT_PUBLIC_API_ORIGIN=$forum NEXT_PUBLIC_FORUM_ORIGIN=$forum \
      NEXT_PUBLIC_WALLET_CHAIN=arbitrum-sepolia pnpm build)
  (cd "$fe/apps/admin" &&
    NEXT_PUBLIC_ADMIN_RP_ID=$admin NEXT_PUBLIC_API_ORIGIN=$forum NEXT_PUBLIC_FORUM_ORIGIN=$forum pnpm build)

  # standalone 保留 monorepo 的目录结构（apps/forum/server.js）；静态资源要另外复制进去
  local sa=$fe/apps/forum/.next/standalone
  [ -f "$sa/apps/forum/server.js" ] || die "no standalone server at $sa/apps/forum/server.js"
  cp -a "$sa" "$dest/forum"
  cp -a "$fe/apps/forum/.next/static" "$dest/forum/apps/forum/.next/static"
  [ -d "$fe/apps/forum/public" ] && cp -a "$fe/apps/forum/public" "$dest/forum/apps/forum/public"
  # forum-web.service 在 current/forum 下运行 node server.js
  printf '%s\n' "// staging release.sh: Next.js standalone keeps the monorepo layout" \
    'require("./apps/forum/server.js");' >"$dest/forum/server.js"
  cp -a "$fe/apps/wallet/out" "$dest/wallet"
  cp -a "$fe/apps/admin/out" "$dest/admin"
  # 产物里不能有本地开发用的东西
  if grep -rlq "akari-dev-only\|NEXT_PUBLIC_DEV_SPONSOR=1" "$dest/wallet" "$dest/admin" "$dest/forum/apps/forum/.next" 2>/dev/null; then
    die "development-only strings found in the frontend build"
  fi
}

build() {
  local base=${1:?usage: release.sh build <sslip base, e.g. 203-0-113-7.sslip.io>}
  [[ "$base" =~ ^[0-9]{1,3}(-[0-9]{1,3}){3}\.sslip\.io$ ]] || die "expected <a-b-c-d>.sslip.io, got $base"
  local rev name stage
  rev=$(cd "$ROOT" && (jj log -r @ --no-graph -T 'change_id.short(8)' 2>/dev/null || git rev-parse --short HEAD))
  name="staging-$(date -u +%Y%m%d%H%M%S)-$rev"
  stage=$OUT/$name
  rm -rf "$stage"
  mkdir -p "$stage"
  build_backend "$stage"
  build_frontend "$base" "$stage"
  tar --zstd -cf "$OUT/$name.tar.zst" -C "$stage" .
  rm -rf "$stage"
  log "built $OUT/$name.tar.zst ($(du -h "$OUT/$name.tar.zst" | cut -f1))"
}

install_release() {
  local ip=${1:?usage: release.sh install <server ip>} artifact name
  artifact=$(ls -1t "$OUT"/staging-*.tar.zst 2>/dev/null | head -1)
  [ -n "$artifact" ] || die "no artifact in $OUT (run build first)"
  name=$(basename "$artifact")
  local ssh=(ssh -i "$SSH_KEY" -o UserKnownHostsFile="$KNOWN_HOSTS" -o StrictHostKeyChecking=yes -o BatchMode=yes)
  log "uploading $name"
  scp -i "$SSH_KEY" -o UserKnownHostsFile="$KNOWN_HOSTS" -o StrictHostKeyChecking=yes -o BatchMode=yes \
    "$artifact" "deploy@$ip:incoming/$name"
  "${ssh[@]}" "deploy@$ip" "sudo /opt/akari/bin/install-release install --unattested /home/deploy/incoming/$name && rm -f /home/deploy/incoming/$name"
}

migrate() {
  local ip=${1:?usage: release.sh migrate <server ip>} sqlx
  sqlx=${SQLX_BIN:-$(command -v sqlx)} || die "sqlx-cli not found (cargo install sqlx-cli --version 0.9.0)"
  local ssh=(ssh -i "$SSH_KEY" -o UserKnownHostsFile="$KNOWN_HOSTS" -o StrictHostKeyChecking=yes -o BatchMode=yes "root@$ip")
  log "migrations: $(find "$ROOT/backend/crates/db/migrations" -name '*.sql' | wc -l) files"
  "${ssh[@]}" 'rm -rf /root/akari-migrate && install -d -m 700 /root/akari-migrate'
  scp -q -i "$SSH_KEY" -o UserKnownHostsFile="$KNOWN_HOSTS" -o StrictHostKeyChecking=yes -o BatchMode=yes \
    -r "$sqlx" "$ROOT/backend/crates/db/migrations" "root@$ip:/root/akari-migrate/"
  # pgroonga 要超级用户建（迁移里的 CREATE EXTENSION IF NOT EXISTS 随后是空操作）；其余对象以 akari_owner 身份创建
  "${ssh[@]}" 'set -e
    cd /tmp
    install -d -m 755 -o postgres /tmp/akari-migrate
    install -m 755 /root/akari-migrate/sqlx /tmp/akari-migrate/sqlx
    cp -r /root/akari-migrate/migrations /tmp/akari-migrate/ && chown -R postgres /tmp/akari-migrate
    runuser -u postgres -- psql -X -q -d akari -c "CREATE EXTENSION IF NOT EXISTS pgroonga"
    runuser -u postgres -- /tmp/akari-migrate/sqlx migrate run --source /tmp/akari-migrate/migrations \
      --database-url "postgres://postgres@%2Frun%2Fpostgresql/akari?options=-c%20role%3Dakari_owner"
    runuser -u postgres -- psql -X -q -d akari -At -c "SELECT max(version) FROM _sqlx_migrations WHERE success"
    rm -rf /tmp/akari-migrate /root/akari-migrate'
}

seed() {
  local ip=${1:?usage: release.sh seed <server ip>} owner_file=${OWNER_EMAIL_FILE:-$HOME/secrets/akari-test-owner.email}
  local ssh=(ssh -i "$SSH_KEY" -o UserKnownHostsFile="$KNOWN_HOSTS" -o StrictHostKeyChecking=yes -o BatchMode=yes "root@$ip")
  local psql="runuser -u postgres -- psql -X -q -d akari -v ON_ERROR_STOP=1"
  "${ssh[@]}" "$psql" <"$ROOT/backend/crates/db/seeds/dev.sql"
  "${ssh[@]}" "$psql" <<'SQL'
INSERT INTO site_settings (key, value) VALUES ('auction', '{"enabled": true}')
ON CONFLICT (key) DO UPDATE SET value = site_settings.value || '{"enabled": true}', updated_at = now();
SQL
  [ -f "$owner_file" ] || { log "no $owner_file: skipping the owner role"; return; }
  # 邮箱只经 stdin 进 psql 变量，不出现在命令行参数与输出里
  local email re
  email=$(tr -d '[:space:]' <"$owner_file" | tr '[:upper:]' '[:lower:]')
  # 不含单引号与反斜杠（psql 的 \set 值在单引号里）
  re=$'^[^\'\\\\]+@[^\'\\\\]+$'
  [[ "$email" =~ $re ]] || die "unexpected email format in $owner_file"
  printf '%s
' "\\set email '$email'" \
    "SELECT CASE WHEN EXISTS (SELECT 1 FROM users WHERE email = :'email') THEN 'owner: user exists' ELSE 'owner: not registered yet (log in once, then rerun seed)' END;" \
    "INSERT INTO staff_roles (user_id, role) SELECT id, 1 FROM users WHERE email = :'email' ON CONFLICT DO NOTHING;" \
    "SELECT 'owner role rows: ' || count(*) FROM staff_roles s JOIN users u ON u.id = s.user_id WHERE u.email = :'email' AND s.role = 1;" |
    "${ssh[@]}" "$psql -At"
}

case "${1:-}" in
  build) shift; build "$@" ;;
  seed) shift; seed "$@" ;;
  install) shift; install_release "$@" ;;
  migrate) shift; migrate "$@" ;;
  *) die "usage: release.sh build <a-b-c-d.sslip.io> | install <server ip> | migrate <server ip> | seed <server ip>" ;;
esac
