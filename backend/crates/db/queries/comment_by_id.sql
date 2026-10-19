-- $1 id
SELECT id, post_id, author_id, reply_to_id, body, status, created_at
FROM floor_comments WHERE id = $1
