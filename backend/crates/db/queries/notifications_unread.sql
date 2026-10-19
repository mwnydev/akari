-- 未读通知数，数到 $3 为止。$1 user_id, $2 已读水位
SELECT count(*) AS "n!" FROM (
  SELECT 1 FROM notifications
  WHERE user_id = $1 AND id > $2 AND created_at > now() - interval '180 days'
  LIMIT $3
) s
