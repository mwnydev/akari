-- $1 id
SELECT p.id, p.topic_id, p.floor, p.author_id, p.html, p.status, p.created_at, p.edited_at,
       p.reply_to_post_id, r.floor AS "reply_to_floor?", r.author_id AS "reply_to_author_id?"
FROM posts p
LEFT JOIN posts r ON r.id = p.reply_to_post_id
WHERE p.id = $1
