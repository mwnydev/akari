-- 已生效（过了冷却期、未撤销）的提币地址：forum-sponsor 只代付转到这些地址的 USDT。$1 user_id, $2 chain_id
SELECT address
FROM withdraw_addresses
WHERE user_id = $1 AND chain_id = $2 AND revoked_at IS NULL AND active_at <= now()
