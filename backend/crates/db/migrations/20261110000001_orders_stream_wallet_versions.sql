-- 订单接口、分期担保（StreamEscrow）索引与钱包初始化参数版本（plan 第 6–8 节、ADR 0008）。
--
-- forum_indexer 另需（deploy/ansible 授权）：
--   GRANT UPDATE (status, updated_at) ON offers TO forum_indexer;   -- 报价成交次数用完即标为已付款
--   GRANT SELECT ON offers TO forum_indexer;

-- ADR 0008：钱包地址由初始化参数版本决定，推算地址、部署钱包都用钱包自己的版本。已有钱包都是第 1 版。
ALTER TABLE wallets ADD COLUMN init_version int NOT NULL DEFAULT 1 CHECK (init_version >= 1);

-- 分期担保合约；旧行由 indexer 启动时写入链配置补上。
ALTER TABLE chains ADD COLUMN stream_address bytea CHECK (octet_length(stream_address) = 20);

ALTER TABLE escrow_orders
  ADD COLUMN started_at  timestamptz,   -- 分期：卖家开始服务的时间
  ADD COLUMN disputed_at timestamptz;   -- 发起争议的区块时间（分期解锁冻结于此，超时兜底从此起算）
COMMENT ON COLUMN escrow_orders.ship_deadline IS '一次性：发货期限；分期：开始期限';
COMMENT ON COLUMN escrow_orders.released_amount IS
  '分期：已领取的可解锁净本金，即合约 Order.claimed（含领取 gas 与从已解锁部分扣的争议费）';

-- 「报价卡片生成的订单」与 indexer 统计报价成交次数。
CREATE INDEX escrow_orders_offer ON escrow_orders (offer_hash);
