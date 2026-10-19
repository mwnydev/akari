-- 举报（同一人对同一内容只记一次，不论之前的案件是否已结案）。
-- $1 target_kind, $2 target_id, $3 forum_id, $4 reporter_id, $5 reason, $6 case_id
INSERT INTO reports (target_kind, target_id, forum_id, reporter_id, reason, case_id)
VALUES ($1, $2, $3, $4, $5, $6)
ON CONFLICT (target_kind, target_id, reporter_id) DO NOTHING
