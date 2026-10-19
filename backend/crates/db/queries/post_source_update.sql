-- $1 post_id, $2 source
UPDATE post_sources SET source = $2 WHERE post_id = $1
