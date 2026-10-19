-- 点赞（幂等）：楼层存在且未删除才写；新赞在同一条语句里给计数加一。需要通知楼主（$3）且这一楼还没有待发的合并通知时，
-- 排一条 $4 秒后投递的 `notify.likes`：窗口内的赞由 worker 投递时合并成一条通知（`likes_digest.sql`）。
-- 已经赞过时不改任何行（`changed` 为假），返回当前计数。$1 post_id, $2 user_id, $3 是否通知楼主, $4 合并窗口（秒）
WITH l AS (
  INSERT INTO post_likes (post_id, user_id)
  SELECT id, $2 FROM posts WHERE id = $1 AND status = 0
  ON CONFLICT DO NOTHING
  RETURNING post_id
), c AS (
  INSERT INTO post_counters AS pc (post_id, likes, like_notify_pending)
  SELECT post_id, 1, $3 FROM l
  ON CONFLICT (post_id) DO UPDATE SET likes = pc.likes + 1, like_notify_pending = pc.like_notify_pending OR $3
  RETURNING pc.likes, old.like_notify_pending AS was_pending
), n AS (
  INSERT INTO outbox (kind, payload, next_attempt_at)
  SELECT 'notify.likes', jsonb_build_object('post_id', $1::bigint), now() + make_interval(secs => $4)
  FROM c
  WHERE $3 AND c.was_pending IS NOT TRUE
)
SELECT EXISTS (SELECT 1 FROM posts WHERE id = $1 AND status = 0) AS "found!", EXISTS (SELECT 1 FROM l) AS "changed!",
       coalesce((SELECT likes FROM c), (SELECT likes FROM post_counters WHERE post_id = $1), 0) AS "likes!"
