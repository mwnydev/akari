#!/usr/bin/env bash
# 安装或回滚一个版本（plan 第 13 节）。部署账号经 sudoers 只能执行本脚本。
#   install-release install <release.tar.zst>   校验 GitHub 构建证明后安装并切换
#   install-release rollback                    切回上一个版本
set -euo pipefail

APP_ROOT=/opt/akari
REPO=mwnydev/akari
KEEP=5
RELEASES="$APP_ROOT/releases"

restart_all() {
  systemctl restart forum-worker forum-indexer forum-sponsor forum-web
  # api 两个实例轮流重启，不停机
  for i in 1 2; do
    systemctl restart "forum-api@$i"
    for _ in $(seq 30); do
      curl -fsS --noproxy '*' "http://127.0.0.1:808$i/api/v1/healthz" >/dev/null 2>&1 && break
      sleep 1
    done
  done
}

switch_to() {
  ln -sfn "$1" "$APP_ROOT/current.new"
  mv -T "$APP_ROOT/current.new" "$APP_ROOT/current"
  restart_all
  echo "current -> $(readlink "$APP_ROOT/current")"
}

case "${1:-}" in
  install)
    artifact=$(realpath -e "${2:?artifact path required}")
    case "$artifact" in /home/deploy/incoming/*.tar.zst) ;; *) echo "artifact must be in /home/deploy/incoming" >&2; exit 2 ;; esac
    gh attestation verify "$artifact" --repo "$REPO" >/dev/null
    name=$(basename "$artifact" .tar.zst)
    [[ "$name" =~ ^[0-9A-Za-z._-]+$ ]] || { echo "bad release name" >&2; exit 2; }
    dest="$RELEASES/$name"
    [ -e "$dest" ] && { echo "$name already installed" >&2; exit 1; }
    mkdir -p "$dest"
    tar --zstd -xf "$artifact" -C "$dest" --no-same-owner
    chown -R root:root "$dest"
    switch_to "$dest"
    # 只保留最近 $KEEP 个版本
    ls -1dt "$RELEASES"/*/ | tail -n +$((KEEP + 1)) | xargs -r rm -rf --
    ;;
  rollback)
    current=$(readlink -f "$APP_ROOT/current")
    prev=$(ls -1dt "$RELEASES"/*/ | sed 's:/$::' | grep -vxF "$current" | head -1)
    [ -n "$prev" ] || { echo "no previous release" >&2; exit 1; }
    switch_to "$prev"
    ;;
  *)
    echo "usage: install-release install <artifact> | rollback" >&2; exit 2 ;;
esac
