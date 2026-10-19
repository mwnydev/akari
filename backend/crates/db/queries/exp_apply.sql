-- 累加经验并按门槛升级；等级不降（plan 第 7 节）。$1 user_id, $2 经验变化
UPDATE user_levels SET
  exp = exp + $2,
  level = greatest(level, (SELECT coalesce(max(t.level), 1::smallint) FROM level_thresholds t WHERE t.min_exp <= exp + $2)),
  updated_at = now()
WHERE user_id = $1
RETURNING level
