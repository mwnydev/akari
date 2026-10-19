-- $1 id, $2 width, $3 height, $4 variants
UPDATE uploads SET status = 'done', width = $2, height = $3, variants = $4, error = NULL, lease_until = NULL,
  updated_at = now()
WHERE id = $1 AND status = 'processing'
