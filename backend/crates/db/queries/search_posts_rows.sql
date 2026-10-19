-- 全文搜索楼层第二步：按 Groonga 给出的 id 回表，只留未删除、主题未隐藏、在访问者可见板块里的楼层。走 posts_pkey、topics_pkey。
-- $1 楼层 id, $2 可见板块 id
-- 列都是 NOT NULL：可空性写死，不依赖 sqlx 按执行计划推断
SELECT p.id AS "id!", p.topic_id AS "topic_id!", p.floor AS "floor!", p.author_id AS "author_id!",
       p.plain_text AS "plain_text!", p.created_at AS "created_at!", t.title AS "title!", t.forum_id AS "forum_id!"
FROM posts p
JOIN topics t ON t.id = p.topic_id
WHERE p.id = ANY($1::bigint[]) AND p.status = 0 AND (t.flags & 8) = 0 AND t.forum_id = ANY($2::int[])
ORDER BY p.id DESC
