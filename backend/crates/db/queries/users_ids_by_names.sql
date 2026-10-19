-- 用户名（已转小写）→ ID，评论里的 @提及用；走唯一索引 users_username。$1 小写用户名
SELECT id FROM users WHERE lower(username) = ANY($1::text[]) AND username IS NOT NULL
