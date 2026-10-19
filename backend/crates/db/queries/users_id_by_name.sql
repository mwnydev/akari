-- 用户名（不区分大小写）→ ID，走唯一索引 users_username。$1 username
SELECT id FROM users WHERE lower(username) = lower($1) AND username IS NOT NULL
