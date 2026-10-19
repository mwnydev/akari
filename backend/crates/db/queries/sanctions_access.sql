-- 登录前检查 IP / 设备封禁。$1 客户端 IP（文本），$2 设备 Cookie 的 SHA-256（可空）
SELECT EXISTS (
         SELECT 1 FROM sanctions
         WHERE kind = 4 AND revoked_at IS NULL AND ip >>= $1::text::inet AND (ends_at IS NULL OR ends_at > now())
       )
    OR EXISTS (
         SELECT 1 FROM sanctions
         WHERE kind = 5 AND revoked_at IS NULL AND device_hash = $2 AND (ends_at IS NULL OR ends_at > now())
       ) AS "banned!"
