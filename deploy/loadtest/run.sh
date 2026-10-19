#!/usr/bin/env bash
# 跑一轮 k6 压测并收集数据库 / Valkey 指标（plan 第 12 节）。forum-api 须已在 BASE 上运行、库里已灌好数据（load.sh）、
# 已生成 targets.json 与 sessions.txt（见 docs/perf/2026-10-load-test.md 的步骤）。
#
#   PSQL='psql postgres://…' VALKEY_CLI='valkey-cli -p 6379' BASE=http://127.0.0.1:8080 \
#     deploy/loadtest/run.sh <名称> [SCALE] [DURATION]
#
# 环境变量：PSQL、VALKEY_CLI（必填）；BASE（默认 http://127.0.0.1:8480）、ORIGIN、WS_CONNS（默认 300）、WS_STORM、K6_MEMORY；DATA（targets.json 与
# sessions.txt 所在目录，默认 .run/loadtest-data）；OUT（结果目录，默认 .run/loadtest-results/<名称>）；
# STATS_CONTAINERS（本机 docker 容器名，空格分隔，采样 CPU / 内存）；STATS_PROCS（进程名，用 pidstat 采样，生产机用）。
# k6 用固定版本的 grafana/k6 镜像，host 网络（只连 BASE）。
set -euo pipefail

NAME="${1:?usage: run.sh <name> [scale] [duration]}"
SCALE="${2:-1}"
DURATION="${3:-5m}"
: "${PSQL:?set PSQL}" "${VALKEY_CLI:?set VALKEY_CLI}"
K6_IMAGE="grafana/k6:1.3.0"
BASE="${BASE:-http://127.0.0.1:8480}"
DATA="$(realpath "${DATA:-.run/loadtest-data}")"
OUT="${OUT:-.run/loadtest-results/$NAME}"
mkdir -p "$OUT"
OUT="$(realpath "$OUT")"
HERE="$(cd "$(dirname "$0")" && pwd)"

q() { $PSQL -v ON_ERROR_STOP=1 -qAt -c "$1" </dev/null; }

q "SELECT pg_stat_statements_reset()" >/dev/null
$VALKEY_CLI CONFIG RESETSTAT >/dev/null

# 资源采样：每 5 秒一次，压测结束后停
sampler() {
  while true; do
    if [[ -n "${STATS_CONTAINERS:-}" ]]; then
      # shellcheck disable=SC2086
      docker stats --no-stream --format '{{.Name}}\t{{.CPUPerc}}\t{{.MemUsage}}' $STATS_CONTAINERS |
        sed "s/^/$(date +%s)\t/"
    fi
    sleep 5
  done
}
sampler > "$OUT/containers.tsv" &
SAMPLER=$!
PIDSTAT=""
if [[ -n "${STATS_PROCS:-}" ]] && command -v pidstat >/dev/null; then
  pidstat -h -u -r -C "$STATS_PROCS" 5 > "$OUT/pidstat.txt" &
  PIDSTAT=$!
fi
cleanup() {
  kill "$SAMPLER" 2>/dev/null || true
  [[ -n "$PIDSTAT" ]] && kill "$PIDSTAT" 2>/dev/null || true
}
trap cleanup EXIT

echo "== k6 $NAME scale=$SCALE duration=$DURATION"
# 本机流量不走代理：只追加本机地址，不改动已有的代理设置
# k6 的每个 WebSocket VU 常驻一个 JS 运行时（约 5–7 MB）：容器限内存，超了只杀 k6，不拖垮压测机（K6_MEMORY，默认 2g）
docker run --rm --network host --user "$(id -u):$(id -g)" --memory "${K6_MEMORY:-2g}" \
  -e NO_PROXY="127.0.0.1,localhost" -e no_proxy="127.0.0.1,localhost" \
  -e BASE="$BASE" -e ORIGIN="${ORIGIN:-http://localhost:3000}" -e SCALE="$SCALE" -e DURATION="$DURATION" \
  -e WS_CONNS="${WS_CONNS:-300}" -e WS_STORM="${WS_STORM:-}" -e OUT=/out/summary.json \
  -v "$HERE/k6:/scripts:ro" -v "$DATA:/data:ro" -v "$OUT:/out" \
  "$K6_IMAGE" run --quiet /scripts/forum.js | tee "$OUT/k6.txt" || true

echo "== slow queries (pg_stat_statements, top 15 by total time)"
q "SELECT round(total_exec_time)::int AS total_ms, calls, round(mean_exec_time::numeric, 2) AS mean_ms,
          round(max_exec_time::numeric, 1) AS max_ms, regexp_replace(left(query, 110), '\s+', ' ', 'g')
   FROM pg_stat_statements WHERE query NOT LIKE '%pg_stat_statements%'
   ORDER BY total_exec_time DESC LIMIT 15" | tee "$OUT/statements.txt"

echo "== database"
q "SELECT 'cache hit ratio', round(100.0 * sum(blks_hit) / nullif(sum(blks_hit + blks_read), 0), 2)::text FROM pg_stat_database
   UNION ALL SELECT 'db size', pg_size_pretty(pg_database_size(current_database()))::text" | tee "$OUT/db.txt"
q "SELECT indexrelname, pg_size_pretty(pg_relation_size(indexrelid)), idx_scan FROM pg_stat_user_indexes
   WHERE relname IN ('posts', 'topics', 'post_likes', 'floor_comments', 'users') ORDER BY pg_relation_size(indexrelid) DESC" |
  tee "$OUT/indexes.txt"

echo "== valkey"
$VALKEY_CLI INFO memory | grep -E '^(used_memory_human|used_memory_peak_human|maxmemory_human)' | tee "$OUT/valkey.txt"
$VALKEY_CLI INFO stats | grep -E '^(keyspace_hits|keyspace_misses|evicted_keys|total_commands_processed|instantaneous_ops_per_sec)' |
  tee -a "$OUT/valkey.txt"
$VALKEY_CLI INFO commandstats | grep -E 'cmdstat_(evalsha|eval|get|hgetall|zrevrange|set|incr):' | tee -a "$OUT/valkey.txt"

if [[ -s "$OUT/containers.tsv" ]]; then
  echo "== container CPU / memory (peak)"
  awk -F'\t' '{cpu=$3; sub(/%/,"",cpu); if (cpu+0 > max[$2]) max[$2]=cpu+0; mem[$2]=$4} END {for (n in max) printf "%s\tpeak CPU %.0f%%\tlast mem %s\n", n, max[n], mem[n]}' \
    "$OUT/containers.tsv" | tee "$OUT/containers-peak.txt"
fi
echo "results in $OUT"
