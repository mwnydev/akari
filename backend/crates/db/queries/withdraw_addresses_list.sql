-- 用户在一条链上的提币地址：未撤销的在前，再按添加先后倒序。$1 user_id, $2 chain_id, $3 条数上限
SELECT id, address, label, active_at, revoked_at, created_at, active_at <= now() AS "active!"
FROM withdraw_addresses
WHERE user_id = $1 AND chain_id = $2
ORDER BY revoked_at IS NOT NULL, id DESC
LIMIT $3
