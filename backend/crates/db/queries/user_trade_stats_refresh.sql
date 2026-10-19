-- 账号交易信誉（plan 第 7 节）：从这些用户最终确认的订单结果重算并写入，只返回实际变化的用户
-- （indexer 据此失效名片缓存；worker 每日全量重算时，返回的就是与链上结果不一致、已被修正的用户）。
-- 防刷：只计 ≥ $2 USDT 的订单；同一对用户（不分买卖方向）每个 30 天窗口（按建单时间对齐）只计 ID 最小的一笔；
-- 自己和自己、钱包不属于本站用户的订单不计。用户的全部订单都在结果里，所以同一对用户的订单一定完整。
-- $1 user ids, $2 计入的最低金额（整 USDT）
WITH o AS (
  SELECT o.id, o.buyer_id, o.seller_id, o.amount,
    escrow_outcome(o.state, o.mode, o.confirm_deadline, o.started_at, o.ruling_to_buyer_bps) AS outcome,
    row_number() OVER (
      PARTITION BY least(o.buyer_id, o.seller_id), greatest(o.buyer_id, o.seller_id),
                   floor(extract(epoch FROM o.created_at) / 2592000)
      ORDER BY o.id
    ) AS nth
  FROM escrow_orders o
  JOIN chains c ON c.chain_id = o.chain_id
  WHERE (o.buyer_id = ANY($1::bigint[]) OR o.seller_id = ANY($1::bigint[]))
    AND o.level = 'finalized' AND o.state IN (3, 4, 5) AND o.buyer_id <> o.seller_id
    AND o.amount >= $2::int * power(10::numeric, c.token_decimals)
), part AS (
  -- 每单拆成卖家、买家两份：(用户, 成功, 本人责任失败为卖家, 为买家, 争议败诉, 和解, 成交额)
  SELECT seller_id AS user_id, outcome IN (1, 4) AS ok, outcome IN (2, 3) AS sf, false AS bf,
         outcome = 3 AS lost, outcome = 5 AS settled, amount
  FROM o WHERE nth = 1
  UNION ALL
  SELECT buyer_id, outcome = 1, false, outcome = 4, outcome = 4, outcome = 5, amount
  FROM o WHERE nth = 1
), agg AS (
  SELECT u.id AS user_id,
    count(*) FILTER (WHERE p.ok)::int AS success,
    count(*) FILTER (WHERE p.sf)::int AS seller_fault_fail,
    count(*) FILTER (WHERE p.bf)::int AS buyer_fault_fail,
    count(*) FILTER (WHERE p.lost)::int AS disputes_lost,
    count(*) FILTER (WHERE p.settled)::int AS settled,
    coalesce(sum(p.amount) FILTER (WHERE p.ok), 0) AS volume
  FROM (SELECT DISTINCT unnest($1::bigint[]) AS id) u
  LEFT JOIN part p ON p.user_id = u.id
  GROUP BY u.id
  -- 没有任何计入订单、也还没有统计行的用户不写（每日全量重算按用户表分批，大多数用户没有订单）
  HAVING count(p.user_id) > 0 OR EXISTS (SELECT 1 FROM user_trade_stats t WHERE t.user_id = u.id)
)
INSERT INTO user_trade_stats AS s
  (user_id, success, seller_fault_fail, buyer_fault_fail, disputes_lost, settled, volume)
SELECT user_id, success, seller_fault_fail, buyer_fault_fail, disputes_lost, settled, volume FROM agg
ON CONFLICT (user_id) DO UPDATE SET
  success = EXCLUDED.success, seller_fault_fail = EXCLUDED.seller_fault_fail,
  buyer_fault_fail = EXCLUDED.buyer_fault_fail, disputes_lost = EXCLUDED.disputes_lost,
  settled = EXCLUDED.settled, volume = EXCLUDED.volume, updated_at = now()
WHERE (s.success, s.seller_fault_fail, s.buyer_fault_fail, s.disputes_lost, s.settled, s.volume)
  IS DISTINCT FROM (EXCLUDED.success, EXCLUDED.seller_fault_fail, EXCLUDED.buyer_fault_fail,
                    EXCLUDED.disputes_lost, EXCLUDED.settled, EXCLUDED.volume)
RETURNING user_id
