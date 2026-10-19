-- 当前用户的注册时间与等级。$1 id
SELECT u.id, u.created_at, coalesce(l.level, 1::smallint) AS "level!"
FROM users u LEFT JOIN user_levels l ON l.user_id = u.id
WHERE u.id = $1
