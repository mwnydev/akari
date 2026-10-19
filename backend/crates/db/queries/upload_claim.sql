-- worker 领取一张待处理图片：排队中的，或处理中但租约已过期的（上次处理的 worker 崩溃或暂时失败后退避）。
-- 不在事务里持锁处理：领取即续租 $1 秒。走 uploads_due。
UPDATE uploads u SET status = 'processing', attempts = u.attempts + 1,
  lease_until = now() + make_interval(secs => $1), updated_at = now()
WHERE u.id = (
  SELECT id FROM uploads
  WHERE status = 'queued' OR (status = 'processing' AND lease_until < now())
  ORDER BY id
  LIMIT 1
  FOR UPDATE SKIP LOCKED
)
RETURNING u.id, u.user_id, u.tmp_key, u.size, u.attempts
