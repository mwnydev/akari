-- 会话列表（有消息的会话，最近的在前）：我作为 user_low / user_high 的两段各走一个索引取前 N 条再合并，
-- 最后一条消息按主键取，未读数在 (conversation_id, id) 索引上数到 100 为止。
-- $1 user_id, $2 游标（上一页最后一个会话的 last_message_id）, $3 条数
SELECT c.id AS "id!",
  CASE WHEN c.user_low = $1 THEN c.user_high ELSE c.user_low END AS "peer_id!",
  CASE WHEN c.user_low = $1 THEN c.low_read_id ELSE c.high_read_id END AS "my_read_id!",
  CASE WHEN c.user_low = $1 THEN c.high_read_id ELSE c.low_read_id END AS "peer_read_id!",
  m.id AS "message_id!", m.sender_id AS "sender_id?", m.kind AS "kind!", m.body AS "body!", m.image AS "image?",
  m.offer_id AS "offer_id?", m.warn AS "warn!", m.created_at AS "created_at!",
  unread.n AS "unread!",
  EXISTS (
    SELECT 1 FROM user_blocks b
    WHERE b.blocker_id = $1 AND b.blocked_id = CASE WHEN c.user_low = $1 THEN c.user_high ELSE c.user_low END
  ) AS "blocked!"
FROM (
  (SELECT * FROM conversations WHERE user_low = $1 AND last_message_id < $2 ORDER BY last_message_id DESC LIMIT $3)
  UNION ALL
  (SELECT * FROM conversations WHERE user_high = $1 AND last_message_id < $2 ORDER BY last_message_id DESC LIMIT $3)
) c
JOIN messages m ON m.id = c.last_message_id
CROSS JOIN LATERAL (
  SELECT count(*) AS n FROM (
    SELECT 1 FROM messages x
    WHERE x.conversation_id = c.id
      AND x.id > CASE WHEN c.user_low = $1 THEN c.low_read_id ELSE c.high_read_id END
      AND x.sender_id IS DISTINCT FROM $1
    LIMIT 100
  ) s
) unread
ORDER BY c.last_message_id DESC
LIMIT $3
