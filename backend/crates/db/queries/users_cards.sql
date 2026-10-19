-- 用户名片批量取（列表页一次取全，禁止 N+1）。$1 ids
SELECT u.id AS "id!", u.username, coalesce(l.level, 1::smallint) AS "level!"
FROM users u
LEFT JOIN user_levels l ON l.user_id = u.id
WHERE u.id = ANY($1::bigint[])
