-- 一页楼层各自最早的 N 条评论（按时间正序），每楼一次部分索引范围扫描。$1 post ids, $2 N
SELECT c.id AS "id!", c.post_id AS "post_id!", c.author_id AS "author_id!", c.reply_to_id, c.body AS "body!",
       c.created_at AS "created_at!"
FROM unnest($1::bigint[]) AS p(id)
CROSS JOIN LATERAL (
  SELECT f.id, f.post_id, f.author_id, f.reply_to_id, f.body, f.created_at
  FROM floor_comments f
  WHERE f.post_id = p.id AND f.status = 0
  ORDER BY f.id
  LIMIT $2
) c
ORDER BY c.post_id, c.id
