-- $1 user_id, $2 content_type, $3 size, $4 tmp_key
INSERT INTO uploads (user_id, content_type, size, tmp_key) VALUES ($1, $2, $3, $4)
RETURNING id, created_at
