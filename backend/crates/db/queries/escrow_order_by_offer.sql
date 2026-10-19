-- 报价卡片生成的、当前用户是买方或卖方的最新一张订单。$1 offer_hash, $2 chain_id, $3 合约地址, $4 用户
SELECT id FROM escrow_orders
WHERE offer_hash = $1 AND chain_id = $2 AND contract = $3 AND state <> 0
  AND (buyer_id = $4 OR seller_id = $4)
ORDER BY id DESC
LIMIT 1
