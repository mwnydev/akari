-- 案件与它的订单（金额以十进制文本取出）。$1 案件 ID，$2 下单快照缺失时的 resolveTimeout 秒数。
SELECT c.id, c.order_id, c.status, c.panel_size, c.evidence_until, c.final_ruling_id, c.published_at, c.appeal_until,
  c.appeal_by, c.appeal_reason, c.appealed_at, c.halted_from, c.halted_by, c.halt_reason, c.ready_at, c.closed_at,
  c.close_reason, c.created_at, c.updated_at,
  o.buyer_id, o.seller_id, o.chain_id, o.contract, o.chain_order_id, o.mode, o.state AS order_state,
  o.amount::text AS "amount!", o.bond::text AS "bond?", o.snapshot->>'net' AS "net?",
  o.released_amount::text AS "released_amount!", ch.token_decimals, o.disputed_at,
  coalesce((o.snapshot->>'resolve_timeout')::int, $2) AS "resolve_timeout_secs!",
  o.ruling_to_buyer_bps AS executed_to_buyer_bps, o.ruling_bond_to_buyer_bps AS executed_bond_to_buyer_bps
FROM arbitration_cases c
JOIN escrow_orders o ON o.id = c.order_id
JOIN chains ch ON ch.chain_id = o.chain_id
WHERE c.id = $1
