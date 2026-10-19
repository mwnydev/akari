-- 某账号在 [from, to) 内的管理操作（不含撤销记录），新的在前。$1 actor_id, $2 from, $3 to, $4 limit
SELECT a.id, a.actor_id, a.action, a.target, a.reason, a.detail, a.created_at, a.reverts,
       EXISTS (SELECT 1 FROM admin_audit_logs r WHERE r.reverts = a.id) AS "reverted!"
FROM admin_audit_logs a
WHERE a.actor_id = $1 AND a.created_at >= $2 AND a.created_at < $3 AND a.reverts IS NULL
ORDER BY a.created_at DESC, a.id DESC
LIMIT $4
