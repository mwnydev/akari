-- 主题列表里拍卖帖的信息行（本页的拍卖帖一次查询）。$1 topic_ids
SELECT id, topic_id AS "topic_id!", state, currency, cny_per_usdt, COALESCE(top_amount, start_price)::text AS "current!",
       bid_count, ends_at AS "ends_at!", COALESCE(top_amount >= reserve_price, false) AS "reserve_met!"
FROM auctions
WHERE topic_id = ANY($1::bigint[])
