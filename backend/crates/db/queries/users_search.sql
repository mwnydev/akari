-- @提及前缀搜索：小写用户名的前缀区间（bytewise 比较，U+10FFFF 不可能出现在用户名里），走 users_username_prefix。
-- $1 前缀, $2 limit
SELECT u.id, u.username AS "username!", u.avatar_url, coalesce(l.level, 1::smallint) AS "level!"
FROM users u
LEFT JOIN user_levels l ON l.user_id = u.id
WHERE u.username IS NOT NULL
  AND lower(u.username) ~>=~ lower($1) AND lower(u.username) ~<~ (lower($1) || chr(1114111))
ORDER BY lower(u.username) USING ~<~
LIMIT $2
