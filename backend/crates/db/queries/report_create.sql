-- 举报（同一人对同一内容只记一次）。$1 target_kind, $2 target_id, $3 forum_id, $4 reporter_id, $5 reason
INSERT INTO reports (target_kind, target_id, forum_id, reporter_id, reason)
VALUES ($1, $2, $3, $4, $5)
ON CONFLICT (target_kind, target_id, reporter_id) DO NOTHING
