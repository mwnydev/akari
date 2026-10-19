-- 深分页：按 (last_post_at, id) 游标，走部分索引 topics_forum_list（排除置顶与隐藏，与 Valkey 有序集合语义一致）。
-- $1 forum_id, $2 before_at, $3 before_id, $4 limit
SELECT id, forum_id, author_id, title, kind, flags, last_floor, last_post_id, last_poster_id,
       last_post_at, created_at, view_count
FROM topics
WHERE forum_id = $1 AND (flags & 9) = 0 AND (last_post_at, id) < ($2, $3)
ORDER BY last_post_at DESC, id DESC
LIMIT $4
