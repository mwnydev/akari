-- 发帖经验：发出满 24 小时的帖子（按主键顺序，从上次处理到的 ID 之后取）。$1 after_id, $2 limit
SELECT p.id, p.author_id, p.floor, p.status, p.plain_text, p.created_at,
       t.author_id AS topic_author_id, t.flags AS topic_flags
FROM posts p
JOIN topics t ON t.id = p.topic_id
WHERE p.id > $1 AND p.created_at <= now() - interval '24 hours'
ORDER BY p.id
LIMIT $2
