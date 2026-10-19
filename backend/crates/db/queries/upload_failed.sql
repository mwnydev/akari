-- $1 id, $2 error
UPDATE uploads SET status = 'failed', error = $2, lease_until = NULL, updated_at = now()
WHERE id = $1 AND status = 'processing'
