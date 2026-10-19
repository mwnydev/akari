-- 结案（只改待处理的），同一条语句给每位举报人排一条通知：只说处理结果（已处理 / 未发现违规），
-- 不含处理方式、处罚内容与处理人。$1 case_id, $2 status, $3 action, $4 resolved_by, $5 topic_id, $6 floor
WITH c AS (
  UPDATE report_cases SET status = $2, action = $3, resolved_by = $4, resolved_at = now()
  WHERE id = $1 AND status = 0
  RETURNING id
), n AS (
  INSERT INTO outbox (kind, payload)
  SELECT 'notify.report_resolved',
         jsonb_build_object('to', r.reporter_id, 'outcome', CASE WHEN $2 = 2 THEN 'actioned' ELSE 'dismissed' END,
                            'topic_id', $5::bigint, 'floor', $6::int)
  FROM c JOIN reports r ON r.case_id = c.id
)
SELECT EXISTS (SELECT 1 FROM c) AS "resolved!"
