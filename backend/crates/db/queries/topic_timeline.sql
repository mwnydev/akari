-- 楼层时间轴的采样点：指定楼层号的发帖时间，走唯一索引 (topic_id, floor)。$1 topic_id, $2 楼层号
SELECT floor, created_at FROM posts
WHERE topic_id = $1 AND floor = ANY($2::int[])
ORDER BY floor
