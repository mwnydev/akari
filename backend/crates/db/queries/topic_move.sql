-- $1 id, $2 forum_id
UPDATE topics t SET forum_id = $2
FROM (SELECT forum_id FROM topics WHERE id = $1 FOR UPDATE) old
WHERE t.id = $1
RETURNING old.forum_id AS "old!"
