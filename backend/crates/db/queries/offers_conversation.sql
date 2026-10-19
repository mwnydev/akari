-- 私信会话里的报价卡片，只有买卖双方可见，新的在前。$1 conversation_id, $2 viewer, $3 limit
SELECT id, listing_id, conversation_id, seller_id, buyer_id, replaces_id, chain_id,
  verifying_contract, seller, buyer, token, amount::text AS "amount!", fee_payer, max_fee_bps,
  max_base_gas::text AS "max_base_gas!", max_min_fee::text AS "max_min_fee!", bond::text AS "bond!",
  late_slash_bps, instant, ship_window, confirm_window, listing_hash, expiry, max_fills,
  nonce::text AS "nonce!", signature, offer_hash, status AS "status: OfferStatus", reject_reason,
  expires_at, created_at, updated_at
FROM offers
WHERE conversation_id = $1 AND (seller_id = $2 OR buyer_id = $2)
ORDER BY id DESC
LIMIT $3
