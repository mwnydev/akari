-- 认领一批到期的待发消息（`FOR UPDATE SKIP LOCKED`：并发的 worker 各取各的）：尝试次数加一（即这次认领的令牌），
-- 到期时间推后一个租期——租期内别的 worker 看不到它，投递者崩溃了租期一过重新到期。
-- 有 `ordering_key` 的只取同一个键里最早未完成的一条（它在投递或退避中时，后面的都等着）。
-- $1 条数, $2 租期（秒）
WITH due AS (
  SELECT id FROM outbox o
  WHERE sent_at IS NULL AND failed_at IS NULL AND next_attempt_at <= now()
    AND (ordering_key IS NULL OR NOT EXISTS (
      SELECT 1 FROM outbox e
      WHERE e.ordering_key = o.ordering_key AND e.id < o.id AND e.sent_at IS NULL AND e.failed_at IS NULL))
  ORDER BY priority DESC, id
  LIMIT $1
  FOR UPDATE SKIP LOCKED
), claimed AS (
  UPDATE outbox SET attempts = attempts + 1, next_attempt_at = now() + make_interval(secs => $2)
  FROM due WHERE outbox.id = due.id
  RETURNING outbox.id, outbox.kind, outbox.payload, outbox.attempts, outbox.priority
)
SELECT id AS "id!", kind AS "kind!", payload AS "payload!", attempts AS "attempts!" FROM claimed ORDER BY priority DESC, id
