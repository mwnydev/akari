-- 会话消息：游标之前的一页，新的在前（调用方再按时间正序返回）。$1 conversation_id, $2 游标（不含）, $3 条数
SELECT id, conversation_id, sender_id, kind, body, image, offer_id, warn, created_at
FROM messages
WHERE conversation_id = $1 AND id < $2
ORDER BY id DESC
LIMIT $3
