-- 锁住用户的等级行（同一用户的经验变动串行，每日上限计数不会被并发绕过）。调用前已确保行存在。$1 user_id
SELECT level, exp FROM user_levels WHERE user_id = $1 FOR UPDATE
