-- 添加地址前的检查（在按用户加锁的事务里）。$1 user_id, $2 chain_id, $3 address
SELECT
  (SELECT count(*) FROM withdraw_addresses WHERE user_id = $1 AND revoked_at IS NULL) AS "live!",
  EXISTS (SELECT 1 FROM withdraw_addresses
          WHERE user_id = $1 AND chain_id = $2 AND address = $3 AND revoked_at IS NULL) AS "duplicate!",
  EXISTS (SELECT 1 FROM withdraw_addresses WHERE user_id = $1) AS "used_before!"
