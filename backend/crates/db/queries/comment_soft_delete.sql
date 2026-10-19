-- 删除评论（只改状态）并给楼层评论数减一；已删除的不再改。返回该楼现在的评论数。$1 id, $2 status（1 作者、2 管理人员）
WITH d AS (
  UPDATE floor_comments SET status = $2 WHERE id = $1 AND status = 0
  RETURNING post_id
)
UPDATE post_counters pc SET comments = pc.comments - 1
FROM d
WHERE pc.post_id = d.post_id
RETURNING pc.comments
