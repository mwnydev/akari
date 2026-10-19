-- 跨板块最新主题超出有序集合之后的深分页，走 topics_latest；只取访问者看得到的板块。
-- $1 可见板块, $2 before_at, $3 before_id, $4 limit
SELECT t.id, t.forum_id, t.author_id, t.title, t.kind, t.flags, t.last_floor, t.last_post_id, t.last_poster_id,
       t.last_post_at, t.created_at, t.view_count, l.id AS "listing_id?", l.price::text AS "price?"
FROM topics t
LEFT JOIN trade_listings l ON l.topic_id = t.id
WHERE (t.flags & 9) = 0 AND t.forum_id = ANY($1::int[]) AND (t.last_post_at, t.id) < ($2, $3)
ORDER BY t.last_post_at DESC, t.id DESC
LIMIT $4
