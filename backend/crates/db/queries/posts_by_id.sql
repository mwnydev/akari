-- $1 id
SELECT id, topic_id, floor, author_id, html, status, created_at, edited_at
FROM posts WHERE id = $1
