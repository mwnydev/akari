-- 恢复被管理人员删除的评论并给楼层评论数加一。返回该楼现在的评论数。$1 id
WITH r AS (
  UPDATE floor_comments SET status = 0 WHERE id = $1 AND status = 2
  RETURNING post_id
)
UPDATE post_counters pc SET comments = pc.comments + 1
FROM r
WHERE pc.post_id = r.post_id
RETURNING pc.comments
