-- 按主键批量取主题（板块列表从 Valkey 取到 ID 后用）；交易帖带上交易帖 ID、价格、参考汇率与发货方式（同一次查询，不逐个取）。$1 ids
SELECT t.id, t.forum_id, t.author_id, t.title, t.kind, t.flags, t.last_floor, t.last_post_id, t.last_poster_id,
       t.last_post_at, t.created_at, t.view_count, t.recent_posters, l.id AS "listing_id?", l.price::text AS "price?",
       l.cny_per_usdt AS "cny_per_usdt?", (l.terms->>'instant')::boolean AS "instant?"
FROM topics t
LEFT JOIN trade_listings l ON l.topic_id = t.id
WHERE t.id = ANY($1::bigint[])
