-- 下一位成交候选：达到保留价、还没拿过成交证明、没被跳过的出价人里出价最高的一口（同价先出的优先）。
-- 出价只会越来越高，所以每人只看最高的那口也就是排在最前的那口。$1 auction_id
SELECT b.id AS bid_id, b.bidder_id, b.amount::text AS "amount!", b.amount_usdt::text AS "amount_usdt!"
FROM auction_bids b
JOIN auctions a ON a.id = b.auction_id
WHERE b.auction_id = $1 AND b.amount >= a.reserve_price AND b.bidder_id <> a.seller_id
  AND NOT EXISTS (SELECT 1 FROM auction_proofs p WHERE p.auction_id = $1 AND p.bidder_id = b.bidder_id)
  AND NOT EXISTS (SELECT 1 FROM auction_passes s WHERE s.auction_id = $1 AND s.bidder_id = b.bidder_id)
ORDER BY b.amount DESC, b.id
LIMIT 1
