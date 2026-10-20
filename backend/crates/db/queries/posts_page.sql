-- 主题第 N 页：按楼层区间取，走唯一索引 (topic_id, floor)，第 1 页与第 5000 页一样快。
-- 被回复楼层（reply_to_post_id）按主键取楼层号与作者，只为显示「回复 #N 名字」。
-- $1 topic_id, $2 from_floor, $3 to_floor
SELECT p.id, p.topic_id, p.floor, p.author_id, p.html, p.status, p.created_at, p.edited_at,
       p.reply_to_post_id, r.floor AS "reply_to_floor?", r.author_id AS "reply_to_author_id?"
FROM posts p
LEFT JOIN posts r ON r.id = p.reply_to_post_id
WHERE p.topic_id = $1 AND p.floor BETWEEN $2 AND $3
ORDER BY p.floor
