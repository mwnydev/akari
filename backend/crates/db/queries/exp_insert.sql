-- 记一条经验流水：同一来源（dedupe）只记一次，有每日上限时当天（来源事件的 UTC 日期）已记满不再记。
-- 调用方已用 exp_lock 锁住该用户。$1 user_id, $2 reason, $3 amount, $4 ref_id, $5 day, $6 dedupe, $7 每日上限（NULL 不限）
INSERT INTO exp_ledger (user_id, reason, amount, ref_id, day, dedupe)
SELECT $1, $2, $3, $4, $5, $6
WHERE $7::int IS NULL
   OR (SELECT count(*) FROM exp_ledger WHERE user_id = $1 AND reason = $2 AND day = $5) < $7
ON CONFLICT (user_id, reason, dedupe) WHERE dedupe IS NOT NULL DO NOTHING
RETURNING id
