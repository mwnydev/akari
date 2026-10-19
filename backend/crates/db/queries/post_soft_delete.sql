-- $1 id, $2 status
UPDATE posts SET status = $2 WHERE id = $1 AND status = 0
