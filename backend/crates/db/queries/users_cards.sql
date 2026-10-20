-- 用户名片批量取（列表页一次取全，禁止 N+1）：用户名、头像、等级与交易信誉（含近 12 个月的拍卖违约与违约禁拍）。$1 ids
SELECT u.id AS "id!", u.username AS "username?", u.avatar_url AS "avatar_url?",
  coalesce(l.level, 1::smallint) AS "level!",
  coalesce(s.success, 0) AS "success!", coalesce(s.seller_fault_fail, 0) AS "seller_fault_fail!",
  coalesce(s.buyer_fault_fail, 0) AS "buyer_fault_fail!", coalesce(s.disputes_lost, 0) AS "disputes_lost!",
  coalesce(s.settled, 0) AS "settled!",
  ad.defaults AS "auction_defaults!", ad.banned_until AS "auction_banned_until?"
FROM users u
LEFT JOIN user_levels l ON l.user_id = u.id
LEFT JOIN user_trade_stats s ON s.user_id = u.id
LEFT JOIN LATERAL (
  SELECT count(*) FILTER (WHERE d.created_at > now() - interval '12 months')::int AS defaults,
    max(x.ends_at) FILTER (WHERE x.revoked_at IS NULL AND x.ends_at > now()) AS banned_until
  FROM auction_defaults d LEFT JOIN sanctions x ON x.id = d.sanction_id
  WHERE d.user_id = u.id
) ad ON true
WHERE u.id = ANY($1::bigint[])
