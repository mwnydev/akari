-- 每件案件最近的 $2 条举报（队列里展示举报理由）。$1 case_ids, $2 每件条数
SELECT r.case_id AS "case_id!", r.reporter_id AS "reporter_id!", r.reason AS "reason!", r.created_at AS "created_at!"
FROM unnest($1::bigint[]) AS c (id)
CROSS JOIN LATERAL (
  SELECT case_id, reporter_id, reason, created_at FROM reports WHERE case_id = c.id ORDER BY id DESC LIMIT $2
) r
