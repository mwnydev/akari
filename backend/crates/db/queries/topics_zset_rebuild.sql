-- 重建板块有序集合：最新的非置顶、非隐藏主题。$1 forum_id, $2 limit
SELECT id, last_post_at
FROM topics
WHERE forum_id = $1 AND (flags & 9) = 0
ORDER BY last_post_at DESC, id DESC
LIMIT $2
