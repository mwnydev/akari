-- 最终确认、已结束订单的结果（定义见 escrow_outcome）与交易经验的判定依据。
-- $1 order ids, $2 完成交易计经验的最低金额（整 USDT）
SELECT o.id, o.buyer_id, o.seller_id,
  escrow_outcome(o.state, o.mode, o.confirm_deadline, o.started_at, o.ruling_to_buyer_bps) AS "outcome!",
  o.amount >= $2::int * power(10::numeric, c.token_decimals) AS "qualifies!",
  floor(extract(epoch FROM o.created_at) / 2592000)::bigint AS "window!",
  (o.created_at AT TIME ZONE 'UTC')::date AS "day!"
FROM escrow_orders o
JOIN chains c ON c.chain_id = o.chain_id
WHERE o.id = ANY($1::bigint[]) AND o.level = 'finalized' AND o.state IN (3, 4, 5)
