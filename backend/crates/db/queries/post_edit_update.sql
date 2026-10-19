-- $1 post_id, $2 html, $3 plain_text
UPDATE posts SET html = $2, plain_text = $3, edited_at = now() WHERE id = $1
