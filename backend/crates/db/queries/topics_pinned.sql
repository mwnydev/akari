-- 板块置顶帖，走部分索引 topics_forum_pinned。$1 forum_id
SELECT id
FROM topics
WHERE forum_id = $1 AND (flags & 1) <> 0 AND (flags & 8) = 0
ORDER BY id DESC
LIMIT 50
