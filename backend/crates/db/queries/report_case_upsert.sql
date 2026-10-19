-- 举报进案件：这一内容有待处理的案件就并进去（计数加一、板块改为内容现在所在的板块），否则开新的一件。
-- 与插入举报行（report_create.sql）在同一事务里；同一人重复举报时整个事务回滚，计数不变。
-- $1 target_kind, $2 target_id, $3 forum_id, $4 target_author_id
INSERT INTO report_cases AS c (target_kind, target_id, forum_id, target_author_id)
VALUES ($1, $2, $3, $4)
ON CONFLICT (target_kind, target_id) WHERE status = 0
DO UPDATE SET reports = c.reports + 1, last_reported_at = now(), forum_id = EXCLUDED.forum_id
RETURNING c.id
