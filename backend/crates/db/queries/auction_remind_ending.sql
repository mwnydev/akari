-- 「即将结束」提醒：进行中、$1 秒内结束的拍卖里，还没收到这条提醒、且在最后 $1 秒之前就出过价的出价人，
-- 一次最多 $2 人；记下已发（kind = $3）并返回，调用方在同一事务里写通知。$1 window_secs $2 limit $3 kind
WITH due AS (
  SELECT b.auction_id, b.bidder_id
  FROM auctions a JOIN auction_bids b ON b.auction_id = a.id
  WHERE a.state = 1 AND a.ends_at > now() AND a.ends_at <= now() + make_interval(secs => $1)
  GROUP BY b.auction_id, b.bidder_id, a.ends_at
  HAVING min(b.created_at) <= a.ends_at - make_interval(secs => $1)
    AND NOT EXISTS (SELECT 1 FROM auction_reminders r
                    WHERE r.auction_id = b.auction_id AND r.kind = $3 AND r.round = 0 AND r.user_id = b.bidder_id)
  LIMIT $2
), sent AS (
  INSERT INTO auction_reminders (auction_id, kind, round, user_id)
  SELECT auction_id, $3, 0, bidder_id FROM due
  ON CONFLICT DO NOTHING
  RETURNING auction_id, user_id
)
SELECT s.auction_id AS "auction_id!", s.user_id AS "user_id!", a.topic_id AS "topic_id?", a.ends_at AS "ends_at!"
FROM sent s JOIN auctions a ON a.id = s.auction_id
