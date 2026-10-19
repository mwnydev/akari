-- 会话消息：游标之后的一页，按时间正序（重连补拉）。$1 conversation_id, $2 游标（不含）, $3 条数
SELECT id, conversation_id, sender_id, kind, body, image, offer_id, warn, created_at
FROM messages
WHERE conversation_id = $1 AND id > $2
ORDER BY id
LIMIT $3
