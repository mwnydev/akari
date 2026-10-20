-- 个人主页：资料、等级与经验、交易信誉（含近 12 个月的拍卖违约与违约禁拍）。$1 id
SELECT u.id, u.username, u.avatar_url, u.created_at, u.last_seen_at, u.show_online,
  coalesce(l.level, 1::smallint) AS "level!", coalesce(l.exp, 0::bigint) AS "exp!",
  coalesce(s.success, 0) AS "success!", coalesce(s.seller_fault_fail, 0) AS "seller_fault_fail!",
  coalesce(s.buyer_fault_fail, 0) AS "buyer_fault_fail!", coalesce(s.disputes_lost, 0) AS "disputes_lost!",
  coalesce(s.settled, 0) AS "settled!", coalesce(s.volume, 0)::text AS "volume!",
  s.avg_reply_secs AS "avg_reply_secs?",
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
WHERE u.id = $1
