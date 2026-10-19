#!/usr/bin/env bash
# 从外部主机（不是源站本身，也不经 Cloudflare WARP）检查源站 IP 是否暴露（docs/runbooks/origin-protection.md）。
#   check-origin-exposure.sh <源站 IP> <域名>
# 每次部署、改防火墙或 DNS、更换源站 IP 后都要跑；有 IPv6 地址时再对 IPv6 跑一次。任何一项失败退出码非 0。
# 依赖：bash、curl、dig（dnsutils / bind-utils）；有 openssl 时额外检查是否送出证书。
# 环境变量：PORTS（要求关闭的 TCP 端口）、SUBDOMAINS（要求不指向源站的子域名）、RESOLVERS（公共 DNS）。
set -uo pipefail

usage() {
  echo "usage: $0 <origin-ip> <domain>" >&2
  exit 2
}
[[ $# -eq 2 ]] || usage
IP=$1
DOMAIN=$2
[[ $IP =~ ^[0-9.]+$ || $IP =~ ^[0-9a-fA-F:]+$ ]] || usage

PORTS=${PORTS:-"22 80 443 2022 5432 6379 7844 8080 8443 22022"}
SUBDOMAINS=${SUBDOMAINS:-"www api ssh mail smtp mx ftp cpanel webmail direct origin staging dev test admin"}
RESOLVERS=${RESOLVERS:-"1.1.1.1 8.8.8.8 9.9.9.9"}
TIMEOUT=5

for tool in curl dig timeout; do
  command -v "$tool" >/dev/null || {
    echo "missing: $tool" >&2
    exit 2
  }
done

# 方括号形式给 curl 与 /dev/tcp 用
if [[ $IP == *:* ]]; then
  URL_HOST="[$IP]"
  RECORD=AAAA
else
  URL_HOST=$IP
  RECORD=A
fi

failures=0
pass() { printf 'PASS  %s\n' "$*"; }
fail() {
  printf 'FAIL  %s\n' "$*"
  failures=$((failures + 1))
}

echo "== TCP ports on $IP (must be closed or filtered)"
for port in $PORTS; do
  # shellcheck disable=SC2016 # 参数由内层 bash 展开
  if timeout "$TIMEOUT" bash -c 'exec 3<>"/dev/tcp/$1/$2"' _ "$IP" "$port" 2>/dev/null; then
    fail "tcp/$port accepts connections"
  else
    pass "tcp/$port closed or filtered"
  fi
done

echo "== Direct HTTP(S) to $IP (must get no HTTP response)"
# 不经代理；-k：即使证书不对也要看能不能拿到响应
direct() {
  curl --noproxy '*' -sk -o /dev/null -w '%{http_code}' --connect-timeout "$TIMEOUT" --max-time $((TIMEOUT * 2)) "$@" 2>/dev/null
}
check_direct() {
  local what=$1
  shift
  local code
  code=$(direct "$@")
  if [[ -n $code && $code != 000 ]]; then
    fail "$what answered HTTP $code"
  else
    pass "$what: no response"
  fi
}
check_direct "https://$IP/ (no SNI)" "https://$URL_HOST/"
check_direct "https://$IP/ with SNI $DOMAIN" --resolve "$DOMAIN:443:$IP" "https://$DOMAIN/"
check_direct "http://$IP/ with Host $DOMAIN" -H "Host: $DOMAIN" "http://$URL_HOST/"

if command -v openssl >/dev/null; then
  # TLS 1.3 下服务器在要求客户端证书之前就送出了自己的证书：送出任何证书都会把域名和源站 IP 关联起来
  for sni in "" "$DOMAIN"; do
    args=(-connect "$URL_HOST:443")
    [[ -n $sni ]] && args+=(-servername "$sni")
    subject=$(timeout "$TIMEOUT" openssl s_client "${args[@]}" </dev/null 2>/dev/null | openssl x509 -noout -subject -ext subjectAltName 2>/dev/null | tr '\n' ' ')
    if [[ -n $subject ]]; then
      fail "TLS on $IP:443 (SNI '${sni:-none}') presents a certificate: $subject"
    else
      pass "TLS on $IP:443 (SNI '${sni:-none}') presents no certificate"
    fi
  done
fi

echo "== Public DNS (must never return $IP)"
names=("$DOMAIN")
for sub in $SUBDOMAINS; do
  names+=("$sub.$DOMAIN")
done
# MX 指向的主机也查
for resolver in $RESOLVERS; do
  while read -r _ mx; do
    [[ -n ${mx:-} ]] && names+=("${mx%.}")
  done < <(dig +short +time=3 +tries=1 MX "$DOMAIN" "@$resolver" 2>/dev/null)
done

mapfile -t names < <(printf '%s\n' "${names[@]}" | sort -u)
before=$failures
for resolver in $RESOLVERS; do
  for name in "${names[@]}"; do
    if dig +short +time=3 +tries=1 "$RECORD" "$name" "@$resolver" 2>/dev/null | grep -Fxqi -- "$IP"; then
      fail "$name $RECORD via $resolver returns the origin IP"
    fi
  done
  if dig +short +time=3 +tries=1 TXT "$DOMAIN" "@$resolver" 2>/dev/null | grep -Fqi -- "$IP"; then
    fail "TXT (SPF etc.) of $DOMAIN via $resolver contains the origin IP"
  fi
done
((failures == before)) && pass "${#names[@]} names ($RECORD) and TXT via $RESOLVERS: origin IP not published"

ptr=$(dig +short +time=3 +tries=1 -x "$IP" 2>/dev/null | tr '\n' ' ')
domain_re=${DOMAIN//./\\.}
if [[ $ptr =~ (^|[. ])${domain_re}\.?( |$) ]]; then
  fail "reverse DNS of $IP names the site: $ptr"
else
  pass "reverse DNS of $IP: ${ptr:-none}"
fi

echo
if ((failures > 0)); then
  echo "$failures check(s) failed: the origin is exposed. See docs/runbooks/origin-protection.md."
  exit 1
fi
echo "All checks passed."
