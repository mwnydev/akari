-- 通知：游标之后的一页，按时间正序（重连补拉）。$1 user_id, $2 游标（不含）, $3 条数, $4 已读水位
SELECT id AS "id!", kind AS "kind!", actor_id, payload AS "payload!", id <= $4 AS "read!", created_at AS "created_at!"
FROM notifications
WHERE user_id = $1 AND id > $2 AND created_at > now() - interval '180 days'
ORDER BY id
LIMIT $3
