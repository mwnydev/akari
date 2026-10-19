-- 合并点赞通知：水位（上一条通知统计到的最后一个赞）之后的新赞，楼主自己的不算；返回人数、最近几位（新的在前）
-- 与最新一个赞的时间（推进水位用）。走 post_likes_recent 只读索引。$1 post_id, $2 水位（可空）, $3 列出几位
SELECT count(*) AS "count!", max(created_at) AS "newest?",
       coalesce((array_agg(user_id ORDER BY created_at DESC))[1:$3], '{}') AS "likers!"
FROM post_likes l
WHERE l.post_id = $1 AND l.created_at > coalesce($2, '-infinity'::timestamptz)
  AND l.user_id <> (SELECT author_id FROM posts WHERE id = $1)
