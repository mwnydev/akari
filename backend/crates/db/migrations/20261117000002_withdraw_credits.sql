-- 免费代付提币次数（plan 第 5 节「gas 规则」）：防止「充值 → 提回」循环白嫖平台代付。
-- 每次代付的 USDT `transfer`（提回交易所）消耗 1 次；开通钱包送 1 次，每笔担保订单到达终态且 finalized 后买卖双方各得 1 次；
-- 每人滚动 24 小时最多代付提币 3 次。没有次数时仍可在「紧急取回」页自付 ETH 提取。
--
-- 余额 = 流水之和，由流水的插入触发器在同一事务里累加到 withdraw_credits（读余额 O(1)，不随流水增长变慢；
-- 计数行只能经流水改动，两者不会不一致）。
--
-- 数据库账号（由 deploy/ansible 创建）另需：
--   GRANT SELECT, INSERT ON withdraw_credit_ledger TO forum_api, forum_indexer, forum_sponsor;
--   GRANT SELECT ON withdraw_credits TO forum_api, forum_sponsor;
--   GRANT SELECT ON sponsorships TO forum_api;          -- 钱包页的「今天还能代付提币几次」
--   GRANT USAGE ON ALL SEQUENCES IN SCHEMA public TO forum_api, forum_indexer, forum_sponsor;
-- 所有账号都不授予 withdraw_credit_ledger 的 UPDATE / DELETE，也不授予 withdraw_credits 的写权限（触发器以属主身份写）。

-- starter：开通钱包（每人 1 次）；order：订单终态 finalized（每单每人 1 次）；
-- withdraw：代付提币签名时扣 1 次；refund_expired：该签名过期且对账确认没有上链（含被取代、代发前作废的）后退回。
CREATE TYPE withdraw_credit_reason AS ENUM ('starter', 'order', 'withdraw', 'refund_expired');

-- 只增流水。
CREATE TABLE withdraw_credit_ledger (
  id             bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  user_id        bigint      NOT NULL REFERENCES users (id),
  delta          smallint    NOT NULL,
  reason         withdraw_credit_reason NOT NULL,
  order_id       bigint      REFERENCES escrow_orders (id),
  sponsorship_id bigint      REFERENCES sponsorships (id),
  created_at     timestamptz NOT NULL DEFAULT now(),
  CHECK (CASE reason
    WHEN 'starter' THEN delta = 1 AND order_id IS NULL AND sponsorship_id IS NULL
    WHEN 'order' THEN delta = 1 AND order_id IS NOT NULL AND sponsorship_id IS NULL
    WHEN 'withdraw' THEN delta = -1 AND order_id IS NULL AND sponsorship_id IS NOT NULL
    ELSE delta = 1 AND order_id IS NULL AND sponsorship_id IS NOT NULL
  END)
);
-- 幂等键：重放、区块回滚后重新折叠、并发写入都不会重复发放或重复扣退。
CREATE UNIQUE INDEX withdraw_credit_ledger_starter ON withdraw_credit_ledger (user_id) WHERE reason = 'starter';
CREATE UNIQUE INDEX withdraw_credit_ledger_order ON withdraw_credit_ledger (order_id, user_id) WHERE reason = 'order';
CREATE UNIQUE INDEX withdraw_credit_ledger_sponsorship ON withdraw_credit_ledger (sponsorship_id, reason)
  WHERE sponsorship_id IS NOT NULL;
-- 「滚动 24 小时最多 3 次」。
CREATE INDEX withdraw_credit_ledger_withdraws ON withdraw_credit_ledger (user_id, created_at) WHERE reason = 'withdraw';

-- 当前余额。重签同一 nonce 时可短暂为负（新旧签名各扣 1 次，最多一笔能上链，没上链的过期后退回）。
CREATE TABLE withdraw_credits (
  user_id    bigint      PRIMARY KEY REFERENCES users (id),
  balance    int         NOT NULL,
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE FUNCTION withdraw_credit_ledger_apply() RETURNS trigger LANGUAGE plpgsql
  SECURITY DEFINER SET search_path = public AS $$
BEGIN
  INSERT INTO withdraw_credits AS c (user_id, balance) VALUES (NEW.user_id, NEW.delta)
  ON CONFLICT (user_id) DO UPDATE SET balance = c.balance + EXCLUDED.balance, updated_at = now();
  RETURN NULL;
END;
$$;
CREATE TRIGGER withdraw_credit_ledger_apply
  AFTER INSERT ON withdraw_credit_ledger
  FOR EACH ROW EXECUTE FUNCTION withdraw_credit_ledger_apply();

CREATE FUNCTION withdraw_credit_ledger_append_only() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'withdraw_credit_ledger is append-only';
END;
$$;
CREATE TRIGGER withdraw_credit_ledger_no_update_delete
  BEFORE UPDATE OR DELETE ON withdraw_credit_ledger
  FOR EACH ROW EXECUTE FUNCTION withdraw_credit_ledger_append_only();
CREATE TRIGGER withdraw_credit_ledger_no_truncate
  BEFORE TRUNCATE ON withdraw_credit_ledger
  FOR EACH STATEMENT EXECUTE FUNCTION withdraw_credit_ledger_append_only();
REVOKE UPDATE, DELETE, TRUNCATE ON withdraw_credit_ledger FROM PUBLIC;

-- 补发：已开通钱包的用户、已经 finalized 的终态订单（3 = 已放款、4 = 已退款、5 = 已结算）。
INSERT INTO withdraw_credit_ledger (user_id, delta, reason)
SELECT DISTINCT user_id, 1, 'starter'::withdraw_credit_reason FROM wallets;
INSERT INTO withdraw_credit_ledger (user_id, delta, reason, order_id)
SELECT DISTINCT p.user_id, 1, 'order'::withdraw_credit_reason, o.id
FROM escrow_orders o CROSS JOIN LATERAL (VALUES (o.buyer_id), (o.seller_id)) AS p (user_id)
WHERE o.state IN (3, 4, 5) AND o.level = 'finalized' AND p.user_id IS NOT NULL;

-- 代发前作废（dropped）的签名在 validUntil 前仍可能经第三方 bundler 上链：与 replaced 一样不算结束，
-- 照占免费额度与提币次数，过期后由对账改为链上结果或 expired（这时才退回）。待上链与对账的部分索引因此包含它。
DROP INDEX sponsorships_open;
DROP INDEX sponsorships_open_until;
CREATE INDEX sponsorships_open ON sponsorships (sender, valid_until)
  WHERE status IN ('signed', 'replaced', 'submitted', 'dropped');
CREATE INDEX sponsorships_open_until ON sponsorships (valid_until)
  WHERE status IN ('signed', 'replaced', 'submitted', 'dropped');
