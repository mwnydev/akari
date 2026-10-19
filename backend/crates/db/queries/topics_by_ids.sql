-- 按主键批量取主题（板块列表从 Valkey 取到 ID 后用）。$1 ids
SELECT id, forum_id, author_id, title, kind, flags, last_floor, last_post_id, last_poster_id,
       last_post_at, created_at, view_count
FROM topics
WHERE id = ANY($1::bigint[])
