#!/usr/bin/env bash
# 把 datagen.py 生成的压测数据用 COPY 灌进一个**已迁移、空的**库（plan 第 12 节压测）。
#
#   PSQL='psql postgres://akari:akari@127.0.0.1:5594/akari' deploy/loadtest/load.sh 2000000
#   PSQL='docker exec -i akari-pg-perf psql -U akari -d akari' deploy/loadtest/load.sh 2000000
#
# 环境变量：PSQL（psql 命令，必填）、OUT（元数据目录，默认 .run/loadtest-data）、JOBS（并行分片数，默认 nproc/2，最多 8）、
# SEED（默认 20261020）。灌数据前删掉大表的二级索引，灌完重建并逐个计时（含 PGroonga），结果写到 $OUT/index-build.tsv。
set -euo pipefail

POSTS="${1:?usage: load.sh <posts>}"
: "${PSQL:?set PSQL to a psql command line}"
OUT="${OUT:-.run/loadtest-data}"
SEED="${SEED:-20261020}"
JOBS="${JOBS:-$(( $(nproc) / 2 ))}"
(( JOBS > 8 )) && JOBS=8
(( JOBS < 1 )) && JOBS=1
HERE="$(cd "$(dirname "$0")" && pwd)"
GEN=(python3 "$HERE/datagen.py" --seed "$SEED" --out "$OUT")

# stdin 留给 COPY：`docker exec -i psql` 这类命令会读走 stdin，单条语句一律不给它
psql_c() { $PSQL -v ON_ERROR_STOP=1 -qAt -c "$1" </dev/null; }

if [[ "$(psql_c 'SELECT count(*) FROM topics')" != 0 ]]; then
  echo "topics is not empty: load into a freshly migrated database" >&2
  exit 1
fi

mkdir -p "$OUT"
echo "== plan ($POSTS posts)"
"${GEN[@]}" plan --posts "$POSTS"

echo "== forums"
psql_c "
INSERT INTO forums (id, slug, name, description, position, view_min_level, post_min_level, reply_min_level, allow_trade, color)
OVERRIDING SYSTEM VALUE VALUES
  (1, 'announcements', '站务公告', '规则变更、活动与维护通知', 0, 0, 99, 1, false, '#d1453b'),
  (2, 'general', '综合讨论', '什么都可以聊', 1, 0, 1, 1, false, '#2f7fd8'),
  (3, 'tech', '技术', '编程、运维、区块链开发', 2, 0, 1, 1, false, '#8059c9'),
  (4, 'help', '帮助', '使用问题与反馈', 3, 0, 1, 1, false, '#1d9bb0'),
  (5, 'lounge', '会员茶馆', '3 级以上可见', 4, 3, 3, 3, false, '#e08e1b'),
  (6, 'trade', '交易区', '担保交易帖', 5, 0, 1, 1, true, '#2e9d5c'),
  (7, 'accounts', '账号交易', '各类账号', 6, 0, 1, 1, true, NULL),
  (8, 'giftcards', '礼品卡', '礼品卡与充值卡', 7, 0, 1, 1, true, NULL),
  (9, 'games', '游戏', '游戏道具与代练', 8, 0, 1, 1, true, NULL),
  (10, 'software', '软件与订阅', '软件授权与订阅', 9, 0, 1, 1, true, NULL),
  (11, 'crypto', '币圈', '行情与链上', 10, 0, 1, 1, false, NULL),
  (12, 'life', '生活', '日常', 11, 0, 1, 1, false, NULL),
  (13, 'jobs', '招聘求职', '工作机会', 12, 0, 1, 1, false, NULL),
  (14, 'feedback', '意见反馈', '建议与问题', 13, 0, 1, 1, false, NULL),
  (15, 'disputes', '争议公示', '仲裁结果', 14, 0, 99, 99, false, NULL),
  (16, 'archive', '旧帖存档', '只读', 15, 0, 99, 99, false, NULL);
SELECT setval(pg_get_serial_sequence('forums', 'id'), 16);"

# 大表的二级索引与 (topic_id, floor) 唯一约束：先删，灌完重建
BULK="'topics','posts','post_sources','users','user_levels','trade_listings','post_counters','post_likes','floor_comments'"
psql_c "SELECT indexdef FROM pg_indexes i
        WHERE tablename IN ($BULK) AND schemaname = 'public'
          AND NOT EXISTS (SELECT 1 FROM pg_constraint c WHERE c.conname = i.indexname)
        ORDER BY (indexdef LIKE '%pgroonga%'), indexname" > "$OUT/indexes.sql"
while read -r def; do
  name="$(sed -E 's/^CREATE (UNIQUE )?INDEX ([^ ]+) .*/\2/' <<<"$def")"
  psql_c "DROP INDEX $name"
done < "$OUT/indexes.sql"
psql_c "ALTER TABLE posts DROP CONSTRAINT posts_topic_id_floor_key"
psql_c "ALTER TABLE trade_listings DROP CONSTRAINT trade_listings_topic_id_key"

copy() { # copy <表(列)> <datagen 子命令> [分片]
  local shard="${3:-0/1}"
  "${GEN[@]}" "$2" --posts "$POSTS" --shard "$shard" | $PSQL -v ON_ERROR_STOP=1 -q -c "COPY $1 FROM STDIN"
}
sharded() { # 按楼层 ID 分 JOBS 片并行灌
  local pids=()
  for ((i = 0; i < JOBS; i++)); do
    copy "$1" "$2" "$i/$JOBS" &
    pids+=($!)
  done
  for p in "${pids[@]}"; do wait "$p"; done
}
timed() {
  local t0=$SECONDS
  "$@"
  echo "   $(( SECONDS - t0 ))s"
}

echo "== users";    timed copy "users (id, email, username, created_at)" users
echo "== levels";   timed copy "user_levels (user_id, level, exp)" levels
echo "== topics";   timed copy "topics (id, forum_id, author_id, title, kind, flags, last_floor, last_post_id, last_poster_id, last_post_at, created_at, view_count, recent_posters)" topics
echo "== listings"; timed copy "trade_listings (id, topic_id, seller_id, chain_id, price, terms, content_hash, created_at, cny_per_usdt)" listings
echo "== posts";    timed sharded "posts (id, topic_id, floor, author_id, html, plain_text, status, created_at)" posts
echo "== sources";  timed sharded "post_sources (post_id, source)" sources
echo "== counters"; timed sharded "post_counters (post_id, likes, comments)" counters
echo "== likes";    timed sharded "post_likes (post_id, user_id, created_at)" likes
echo "== comments"; timed sharded "floor_comments (id, post_id, author_id, reply_to_id, body, status, created_at)" comments

echo "== sequences"
for t in users topics posts trade_listings floor_comments; do
  psql_c "SELECT setval(pg_get_serial_sequence('$t', 'id'), (SELECT max(id) FROM $t))" >/dev/null
done

echo "== indexes"
: > "$OUT/index-build.tsv"
build() { # build <名称> <SQL>
  local t0
  t0=$(date +%s%3N)
  psql_c "$2"
  printf '%s\t%s ms\n' "$1" "$(( $(date +%s%3N) - t0 ))" | tee -a "$OUT/index-build.tsv"
}
build posts_topic_id_floor_key "ALTER TABLE posts ADD CONSTRAINT posts_topic_id_floor_key UNIQUE (topic_id, floor)"
build trade_listings_topic_id_key "ALTER TABLE trade_listings ADD CONSTRAINT trade_listings_topic_id_key UNIQUE (topic_id)"
while read -r def; do
  name="$(sed -E 's/^CREATE (UNIQUE )?INDEX ([^ ]+) .*/\2/' <<<"$def")"
  build "$name" "SET maintenance_work_mem = '1GB'; $def"
done < "$OUT/indexes.sql"

echo "== vacuum analyze"
timed psql_c "VACUUM (ANALYZE) users, user_levels, topics, trade_listings, posts, post_sources, post_counters, post_likes, floor_comments"
psql_c "SELECT relname, pg_size_pretty(pg_total_relation_size(oid)) FROM pg_class
        WHERE relname IN ($BULK) ORDER BY pg_total_relation_size(oid) DESC"
