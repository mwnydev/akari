-- 重建跨板块最新主题有序集合：全站最新的非置顶、非隐藏主题，走 topics_latest。$1 limit
SELECT id, last_post_at
FROM topics
WHERE (flags & 9) = 0
ORDER BY last_post_at DESC, id DESC
LIMIT $1
