-- 暂时失败（存储不可用等）：保持处理中，租约改为 $2 秒后到期，到期后重新领取。$1 id, $2 秒, $3 error
UPDATE uploads SET lease_until = now() + make_interval(secs => $2), error = $3, updated_at = now()
WHERE id = $1 AND status = 'processing'
