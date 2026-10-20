-- 一场拍卖的全部字段。$1 id
SELECT id, seller_id, forum_id, topic_id, chain_id, verifying_contract, seller, currency, cny_per_usdt,
       start_price::text AS "start_price!", increment::text AS "increment!", reserve_price::text AS "reserve_price!",
       duration_secs, eligibility, eligibility_value::text AS "eligibility_value?", terms, content_hash, draft, auth,
       auth_hash, sign_deadline, seller_signature, snipe_window_secs, snipe_extension_secs, extension_cap_secs, state,
       starts_at, ends_at, max_ends_at, top_amount::text AS "top_amount?", top_bidder_id, bid_count, round,
       pay_deadline, order_id, created_at
FROM auctions
WHERE id = $1
