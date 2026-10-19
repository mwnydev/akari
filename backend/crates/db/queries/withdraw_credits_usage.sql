-- 代付提币次数：余额、滚动窗口内已用的代付提币、同一账户同一 nonce 仍未结束的旧签名所扣的次数。
-- 已用次数与免费额度一样只数可能或已经上链的签名：被取代、代发前作废的在对账确认过期前照算（validUntil 前仍可能经
-- 第三方上链），同一 nonce 的多笔签名最多一笔能上链，只算一次；同一 nonce 待取代的旧签名不算，重签不会多占当天的次数。
-- $3–$5 为空时（只看余额与当天次数）后两项按无同 nonce 签名处理。
SELECT
  coalesce((SELECT balance FROM withdraw_credits WHERE user_id = $1), 0)::bigint AS "balance!",
  (SELECT count(DISTINCT (s.chain_id, s.sender, s.nonce)) FROM withdraw_credit_ledger l
   JOIN sponsorships s ON s.id = l.sponsorship_id
   WHERE l.user_id = $1 AND l.reason = 'withdraw' AND l.created_at > $2
     AND s.status <> 'expired'
     AND (s.chain_id = $3 AND s.sender = $4 AND s.nonce = $5::text::numeric
          AND s.status IN ('signed', 'replaced', 'dropped')) IS NOT TRUE
  ) AS "today!",
  (SELECT count(*) FROM sponsorships s
   JOIN withdraw_credit_ledger l ON l.sponsorship_id = s.id AND l.reason = 'withdraw'
   WHERE s.sender = $4 AND s.chain_id = $3 AND s.nonce = $5::text::numeric
     AND s.status IN ('signed', 'replaced', 'dropped')
  ) AS "held!"
