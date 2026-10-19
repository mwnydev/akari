-- 某账号最近一段时间内指定操作的次数；被站长恢复过管理权限的，只算恢复之后的。$1 actor_id, $2 actions, $3 窗口秒数
SELECT count(*) AS "count!" FROM admin_audit_logs
WHERE actor_id = $1 AND action = ANY($2::text[])
  AND created_at > greatest(
        now() - make_interval(secs => $3),
        (SELECT max(reinstated_at) FROM staff_suspensions WHERE user_id = $1)
      )
