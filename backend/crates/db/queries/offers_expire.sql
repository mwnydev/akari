-- worker：到期的待确认卡片标为已过期，一批最多 $1 条；多个 worker 并发时跳过已锁行。
UPDATE offers SET status = 'expired', updated_at = now()
WHERE id IN (
  SELECT id FROM offers
  WHERE status = 'pending' AND expires_at <= now()
  ORDER BY expires_at
  LIMIT $1
  FOR UPDATE SKIP LOCKED
)
