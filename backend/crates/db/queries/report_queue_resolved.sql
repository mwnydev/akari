-- 举报队列（已结案）：每个板块从部分索引 report_cases_resolved 取最新的 $3 件再合并（板块只有几十个），
-- 不含访问者自己的内容。$1 forum_ids, $2 游标（上一页最后一件的 ID，不含）, $3 limit, $4 访问者
SELECT c.id AS "id!", c.target_kind AS "target_kind!", c.target_id AS "target_id!", c.forum_id AS "forum_id!",
       c.target_author_id AS "target_author_id!", c.reports AS "reports!", c.last_reported_at AS "last_reported_at!",
       c.created_at AS "created_at!", c.status AS "status!", c.action, c.resolved_by, c.resolved_at
FROM unnest($1::int[]) AS f (id)
CROSS JOIN LATERAL (
  SELECT r.id, r.target_kind, r.target_id, r.forum_id, r.target_author_id, r.reports, r.last_reported_at,
         r.created_at, r.status, r.action, r.resolved_by, r.resolved_at
  FROM report_cases r
  WHERE r.forum_id = f.id AND r.status <> 0 AND r.id < $2 AND r.target_author_id <> $4
  ORDER BY r.id DESC
  LIMIT $3
) c
ORDER BY c.id DESC
LIMIT $3
