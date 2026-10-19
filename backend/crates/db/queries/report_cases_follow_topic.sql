-- 主题移动后，它的楼层与评论上待处理的案件跟到新板块（新板块的版主处理）。只扫待处理的案件（很少），
-- 按主键回查楼层 / 评论。$1 topic_id, $2 新板块
UPDATE report_cases rc SET forum_id = $2
WHERE rc.status = 0 AND rc.forum_id <> $2
  AND EXISTS (
    SELECT 1 FROM posts p
    WHERE p.topic_id = $1
      AND p.id = CASE rc.target_kind
                   WHEN 1 THEN rc.target_id
                   ELSE (SELECT fc.post_id FROM floor_comments fc WHERE fc.id = rc.target_id)
                 END
  )
