-- 一楼的评论翻页（「展开其余」）：游标之后的一页，按时间正序。$1 post_id, $2 游标（不含）, $3 条数
SELECT id, post_id, author_id, reply_to_id, body, created_at
FROM floor_comments
WHERE post_id = $1 AND status = 0 AND id > $2
ORDER BY id
LIMIT $3
