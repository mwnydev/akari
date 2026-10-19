-- 浏览器直传完成：待上传 → 排队处理（只有上传者本人能确认，重复确认不再排队）。$1 id, $2 user_id
UPDATE uploads SET status = 'queued', updated_at = now()
WHERE id = $1 AND user_id = $2 AND status = 'pending'
RETURNING id
