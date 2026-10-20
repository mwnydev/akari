-- 每种消息的待发与死信条数（不读 payload）：待发走 outbox_pending，死信走 outbox_dead，已发送的行不碰。
SELECT kind AS "kind!",
       count(*) FILTER (WHERE failed_at IS NULL) AS "pending!",
       count(*) FILTER (WHERE failed_at IS NULL AND attempts > 0) AS "retrying!",
       count(*) FILTER (WHERE failed_at IS NOT NULL) AS "dead!",
       min(created_at) FILTER (WHERE failed_at IS NULL) AS oldest_pending
FROM (
  SELECT kind, attempts, failed_at, created_at FROM outbox WHERE sent_at IS NULL AND failed_at IS NULL
  UNION ALL
  SELECT kind, attempts, failed_at, created_at FROM outbox WHERE failed_at IS NOT NULL AND discarded_at IS NULL
) t
GROUP BY kind ORDER BY kind
