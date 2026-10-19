-- 主题第 N 页：按楼层区间取，走唯一索引 (topic_id, floor)，第 1 页与第 5000 页一样快。
-- $1 topic_id, $2 from_floor, $3 to_floor
SELECT id, topic_id, floor, author_id, html, status, created_at, edited_at
FROM posts
WHERE topic_id = $1 AND floor BETWEEN $2 AND $3
ORDER BY floor
