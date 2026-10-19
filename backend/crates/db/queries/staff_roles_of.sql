-- 一个账号的管理角色与是否被暂停；不是管理人员时只有一行（role 为空）。$1 user_id
SELECT r.role AS "role?", r.forum_id AS "forum_id?",
       EXISTS (SELECT 1 FROM staff_suspensions s WHERE s.user_id = $1 AND s.reinstated_at IS NULL) AS "suspended!"
FROM (SELECT 1) one
LEFT JOIN staff_roles r ON r.user_id = $1
