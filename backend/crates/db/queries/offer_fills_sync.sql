-- 报价卡片随链上成交同步（indexer 在写订单的同一事务里调用）：未回滚的订单数达到可成交次数即标为已付款；
-- 区块回滚后不再用完的已付款卡片恢复为待确认（已过有效期的为已过期）。回滚前若在站内被取消、修改或拒绝过，
-- 恢复后也按待确认显示：链上签名本来就仍然有效，这种情况极少见。$1 chain_id, $2 合约地址, $3 offer_hash
WITH fills AS (
  SELECT count(*) AS n FROM escrow_orders
  WHERE offer_hash = $3 AND chain_id = $1 AND contract = $2 AND state <> 0
)
UPDATE offers f SET
  status = CASE
    WHEN fills.n >= f.max_fills THEN 'paid'::offer_status
    WHEN f.expires_at <= now() THEN 'expired'::offer_status
    ELSE 'pending'::offer_status
  END,
  updated_at = now()
FROM fills
WHERE f.offer_hash = $3 AND f.chain_id = $1 AND f.verifying_contract = $2
  AND (fills.n >= f.max_fills) <> (f.status = 'paid')
