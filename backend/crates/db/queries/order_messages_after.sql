-- 订单聊天：游标之后的一页，按时间正序（重连补拉、哈希链校验）。$1 order_id, $2 游标（不含）, $3 条数
SELECT id, order_id, sender_id, kind, body, image, warn, prev_hash, hash, created_at
FROM order_messages
WHERE order_id = $1 AND id > $2
ORDER BY id
LIMIT $3
