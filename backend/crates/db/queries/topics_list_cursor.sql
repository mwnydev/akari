-- 深分页：按 (last_post_at, id) 游标，走部分索引 topics_forum_list（排除置顶与隐藏，与 Valkey 有序集合语义一致）。
-- $1 forum_id, $2 before_at, $3 before_id, $4 limit
SELECT t.id, t.forum_id, t.author_id, t.title, t.kind, t.flags, t.last_floor, t.last_post_id, t.last_poster_id,
       t.last_post_at, t.created_at, t.view_count, t.recent_posters, l.id AS "listing_id?", l.price::text AS "price?",
       l.cny_per_usdt AS "cny_per_usdt?", (l.terms->>'instant')::boolean AS "instant?"
FROM topics t
LEFT JOIN trade_listings l ON l.topic_id = t.id
WHERE t.forum_id = $1 AND (t.flags & 9) = 0 AND (t.last_post_at, t.id) < ($2, $3)
ORDER BY t.last_post_at DESC, t.id DESC
LIMIT $4
