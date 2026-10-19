-- 用户主页的帖子，新的在前，游标分页，走 posts_author。只含未删除、所在主题未隐藏、访问者看得到的板块里的楼层。
-- $1 author_id, $2 before_id, $3 可见板块, $4 limit
SELECT p.id, p.topic_id, p.floor, left(p.plain_text, 200) AS "excerpt!", p.created_at,
       t.title, t.forum_id
FROM posts p
JOIN topics t ON t.id = p.topic_id
WHERE p.author_id = $1 AND p.id < $2 AND p.status = 0
  AND (t.flags & 8) = 0 AND t.forum_id = ANY($3::int[])
ORDER BY p.id DESC
LIMIT $4
