-- 个人主页：资料、等级与经验、交易信誉。$1 id
SELECT u.id, u.username, u.avatar_url, u.created_at, u.last_seen_at, u.show_online,
  coalesce(l.level, 1::smallint) AS "level!", coalesce(l.exp, 0::bigint) AS "exp!",
  coalesce(s.success, 0) AS "success!", coalesce(s.seller_fault_fail, 0) AS "seller_fault_fail!",
  coalesce(s.buyer_fault_fail, 0) AS "buyer_fault_fail!", coalesce(s.disputes_lost, 0) AS "disputes_lost!",
  coalesce(s.settled, 0) AS "settled!", coalesce(s.volume, 0)::text AS "volume!",
  s.avg_reply_secs AS "avg_reply_secs?"
FROM users u
LEFT JOIN user_levels l ON l.user_id = u.id
LEFT JOIN user_trade_stats s ON s.user_id = u.id
WHERE u.id = $1
