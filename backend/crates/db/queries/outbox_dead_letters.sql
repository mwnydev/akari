-- 未丢弃的死信（不含 payload），新的在前。$1 类型（NULL 不限）, $2 上一页最后一条的 ID（NULL 第一页）, $3 条数
SELECT id, kind, attempts, last_error, created_at, failed_at AS "failed_at!"
FROM outbox
WHERE failed_at IS NOT NULL AND discarded_at IS NULL
  AND ($1::text IS NULL OR kind = $1) AND ($2::bigint IS NULL OR id < $2)
ORDER BY id DESC
LIMIT $3
