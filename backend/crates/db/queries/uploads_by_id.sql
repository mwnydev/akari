-- $1 id
SELECT id, user_id, status AS "status: UploadStatus", content_type, size, tmp_key, width, height, variants,
       error, attempts, created_at, updated_at
FROM uploads WHERE id = $1
