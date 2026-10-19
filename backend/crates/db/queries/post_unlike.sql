-- 取消赞（幂等）：删掉赞并在同一条语句里给计数减一；没赞过时不改任何行（`changed` 为假）。返回当前计数。$1 post_id, $2 user_id
WITH d AS (
  DELETE FROM post_likes WHERE post_id = $1 AND user_id = $2
  RETURNING post_id
), c AS (
  UPDATE post_counters pc SET likes = pc.likes - 1
  FROM d
  WHERE pc.post_id = d.post_id
  RETURNING pc.likes
)
SELECT EXISTS (SELECT 1 FROM d) AS "changed!",
       coalesce((SELECT likes FROM c), (SELECT likes FROM post_counters WHERE post_id = $1), 0) AS "likes!"
