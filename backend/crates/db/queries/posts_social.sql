-- 一页楼层的点赞数、评论数、最近点赞的几个人（新的在前）与访问者是否已赞：每楼两次主键 / 只读索引查找。
-- $1 post ids, $2 访问者（游客为 0）, $3 点赞名单人数
SELECT p.id AS "post_id!", coalesce(c.likes, 0) AS "likes!", coalesce(c.comments, 0) AS "comments!",
       ARRAY(
         SELECT l.user_id FROM post_likes l WHERE l.post_id = p.id ORDER BY l.created_at DESC LIMIT $3
       ) AS "likers!",
       EXISTS (SELECT 1 FROM post_likes v WHERE v.post_id = p.id AND v.user_id = $2) AS "liked!"
FROM unnest($1::bigint[]) AS p(id)
LEFT JOIN post_counters c ON c.post_id = p.id
