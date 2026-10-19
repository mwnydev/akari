-- 交易帖的剩余库存（主题列表）：仍有效的公开报价（指定买家的不算）的可成交次数减去已成交（未回滚）的订单数，按交易帖合计。
-- 没有有效公开报价的交易帖不返回。走 offers_listing 与 escrow_orders_offer。$1 listing ids
SELECT o.listing_id AS "listing_id!",
       sum(o.max_fills - (
         SELECT count(*) FROM escrow_orders e
         WHERE e.offer_hash = o.offer_hash AND e.chain_id = o.chain_id AND e.contract = o.verifying_contract
           AND e.state <> 0
       ))::bigint AS "stock!"
FROM offers o
WHERE o.listing_id = ANY($1::bigint[]) AND o.buyer_id IS NULL AND o.status = 'pending' AND o.expires_at > now()
GROUP BY o.listing_id
