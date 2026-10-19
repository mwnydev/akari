-- 按主键批量取订单（金额以十进制文本取出），附带生成它的报价卡片与交易帖。$1 ids
SELECT o.id, o.chain_id, o.contract, o.chain_order_id, o.mode, o.buyer, o.seller, o.buyer_id, o.seller_id,
  o.token, o.amount::text AS "amount!", o.buyer_fee::text AS "buyer_fee?", o.seller_fee::text AS "seller_fee?",
  o.buyer_gas::text AS "buyer_gas?", o.seller_gas::text AS "seller_gas?", o.dispute_fee::text AS "dispute_fee?",
  o.bond::text AS "bond?", o.instant, o.offer_hash, o.listing_hash, o.auction_id::text AS "auction_id?", o.state,
  o.ship_deadline, o.confirm_deadline, o.proposer, o.proposal_to_buyer::text AS "proposal_to_buyer?",
  o.proposal_expiry, o.disputed_by, o.dispute_from_bond, o.disputed_at, o.ruling_to_buyer_bps,
  o.ruling_bond_to_buyer_bps, o.settled_to_buyer::text AS "settled_to_buyer?",
  o.settled_to_seller::text AS "settled_to_seller?", o.settled_to_fee::text AS "settled_to_fee?",
  o.interval_secs, o.periods, o.started_at, o.released_amount::text AS "released_amount!",
  o.level::text AS "level!", o.created_block, o.updated_block, o.created_at, o.updated_at,
  f.id AS "offer_id?", f.listing_id AS "listing_id?", l.topic_id AS "listing_topic_id?"
FROM escrow_orders o
LEFT JOIN offers f ON f.offer_hash = o.offer_hash AND f.chain_id = o.chain_id AND f.verifying_contract = o.contract
LEFT JOIN trade_listings l ON l.id = f.listing_id
WHERE o.id = ANY($1::bigint[])
ORDER BY o.id DESC
