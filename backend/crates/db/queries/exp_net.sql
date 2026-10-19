-- 某一来源累计给出的经验（精华：设精华 +50、撤销 −50，净值决定下一步能否再加或扣回）。$1 user_id, $2 reason, $3 ref_id
SELECT coalesce(sum(amount), 0)::bigint AS "net!"
FROM exp_ledger
WHERE user_id = $1 AND reason = $2 AND ref_id = $3
