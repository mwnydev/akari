-- 生效中的处罚里编号最大的一种（封号 3 优先于禁言 1）。$1 user_id, $2 kinds, $3 forum_id（空 = 只看全站）
SELECT kind FROM sanctions
WHERE user_id = $1 AND revoked_at IS NULL AND kind = ANY($2::smallint[])
  AND (ends_at IS NULL OR ends_at > now())
  AND (forum_id IS NULL OR forum_id = $3)
ORDER BY kind DESC
LIMIT 1
