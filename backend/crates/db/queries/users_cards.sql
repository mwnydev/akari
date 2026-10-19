-- 用户名片批量取（列表页一次取全，禁止 N+1）：用户名、头像、等级与交易信誉。$1 ids
SELECT u.id AS "id!", u.username AS "username?", u.avatar_url AS "avatar_url?",
  coalesce(l.level, 1::smallint) AS "level!",
  coalesce(s.success, 0) AS "success!", coalesce(s.seller_fault_fail, 0) AS "seller_fault_fail!",
  coalesce(s.buyer_fault_fail, 0) AS "buyer_fault_fail!", coalesce(s.disputes_lost, 0) AS "disputes_lost!",
  coalesce(s.settled, 0) AS "settled!"
FROM users u
LEFT JOIN user_levels l ON l.user_id = u.id
LEFT JOIN user_trade_stats s ON s.user_id = u.id
WHERE u.id = ANY($1::bigint[])
