-- 付款提醒：待付款、$1 秒内到付款截止时间、这一轮还没提醒过的拍卖（证明签发时离截止已不到 $1 秒的除外），
-- 一次最多 $2 场；记下已发（kind = $3）并返回这一轮的中标者，调用方在同一事务里写通知与邮件。
-- $1 window_secs $2 limit $3 kind
WITH due AS (
  SELECT a.id AS auction_id, a.topic_id, a.round, p.bidder_id, p.pay_deadline
  FROM auctions a JOIN auction_proofs p ON p.auction_id = a.id AND p.round = a.round
  WHERE a.state = 2 AND a.pay_deadline > now() AND a.pay_deadline <= now() + make_interval(secs => $1)
    AND p.pay_deadline - p.created_at > make_interval(secs => $1)
    AND NOT EXISTS (SELECT 1 FROM auction_reminders r WHERE r.auction_id = a.id AND r.kind = $3 AND r.round = a.round)
  ORDER BY a.pay_deadline
  LIMIT $2
), sent AS (
  INSERT INTO auction_reminders (auction_id, kind, round, user_id)
  SELECT auction_id, $3, round, bidder_id FROM due
  ON CONFLICT DO NOTHING
  RETURNING auction_id, round
)
SELECT d.auction_id AS "auction_id!", d.topic_id AS "topic_id?", d.round AS "round!", d.bidder_id AS "user_id!",
  d.pay_deadline AS "pay_deadline!"
FROM due d JOIN sent s ON s.auction_id = d.auction_id AND s.round = d.round
