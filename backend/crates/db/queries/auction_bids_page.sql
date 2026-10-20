-- 出价记录翻页（新的在前），带出价人用户名（接口层打码）。$1 auction_id, $2 before（不含）, $3 limit
SELECT b.id, b.bidder_id, u.username, b.amount::text AS "amount!", b.linked, b.created_at
FROM auction_bids b
JOIN users u ON u.id = b.bidder_id
WHERE b.auction_id = $1 AND b.id < $2
ORDER BY b.id DESC
LIMIT $3
