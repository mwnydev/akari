-- 设置用户名：首次设置不限，之后 30 天只能改一次（条件 UPDATE，并发下不会绕过）。重名由唯一索引 users_username 拒绝。
-- $1 id, $2 username
UPDATE users SET username = $2, username_changed_at = now()
WHERE id = $1
  AND (username IS NULL OR username_changed_at IS NULL OR username_changed_at <= now() - interval '30 days')
RETURNING id
