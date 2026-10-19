-- 链上派生表（plan 第 2、8 节）：只有 forum-indexer 写入，其他程序只读。
--
-- 数据库账号（由 deploy/ansible 创建，见扩展迁移顶部说明）应授予：
--   GRANT SELECT, INSERT, UPDATE, DELETE ON chains, escrow_orders, escrow_events,
--         indexer_cursors, indexer_checkpoints TO forum_indexer;
--   GRANT INSERT ON outbox TO forum_indexer;          -- 每次订单状态变化写一条 order.updated
--   GRANT SELECT ON wallets TO forum_indexer;         -- 钱包地址 → 用户
--   GRANT USAGE ON ALL SEQUENCES IN SCHEMA public TO forum_indexer;
--   GRANT SELECT ON chains, escrow_orders, escrow_events TO forum_api, forum_worker, forum_sponsor;
--   REVOKE INSERT, UPDATE, DELETE, TRUNCATE ON chains, escrow_orders, escrow_events,
--          indexer_cursors, indexer_checkpoints FROM forum_api, forum_worker, forum_sponsor;
--
-- 地址存 20 字节、哈希存 32 字节 bytea；链上金额 NUMERIC(38,0) 存最小单位（plan 第 8 节规则）。
--
-- escrow_events 暂不按月分区：分区表的唯一约束必须包含分区键，(chain_id, tx_hash, log_index) 唯一就要求
-- 每次写入前先知道区块时间（不是所有节点的日志都带 blockTimestamp）。按 plan 第 12 节的交易量（每单约 3–6 条
-- 事件）未来几年都在千万行以内，block_number 上的 BRIN 足够；需要时再新增迁移改为分区表。

-- 确认级别：顺序即确认程度，可以直接用 min() / 比较。
CREATE TYPE chain_level AS ENUM ('latest', 'safe', 'finalized');

-- 链配置的公开部分（由 indexer 启动时从链配置文件写入）。RPC 地址带服务商密钥，只在配置文件 / 环境变量里。
CREATE TABLE chains (
  chain_id         bigint      PRIMARY KEY,
  name             text        NOT NULL,
  escrow_address   bytea       NOT NULL CHECK (octet_length(escrow_address) = 20),
  config_address   bytea       NOT NULL CHECK (octet_length(config_address) = 20),
  token_address    bytea       NOT NULL CHECK (octet_length(token_address) = 20),
  token_symbol     text        NOT NULL,
  token_decimals   smallint    NOT NULL CHECK (token_decimals BETWEEN 0 AND 36),
  start_block      bigint      NOT NULL,
  safe_depth       int         NOT NULL,   -- 节点不支持 safe 标记时的确认块数
  finalized_depth  int         NOT NULL,   -- 节点不支持 finalized 标记时的确认块数
  updated_at       timestamptz NOT NULL DEFAULT now()
);

-- 原始合约日志（Escrow 与 EscrowConfig 的全部事件）。区块回滚时删除未最终确认的行再重放。
CREATE TABLE escrow_events (
  id           bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  chain_id     bigint      NOT NULL REFERENCES chains (chain_id),
  contract     bytea       NOT NULL CHECK (octet_length(contract) = 20),
  block_number bigint      NOT NULL,
  block_hash   bytea       NOT NULL CHECK (octet_length(block_hash) = 32),
  block_time   timestamptz,                         -- 节点日志带 blockTimestamp 时才有
  tx_hash      bytea       NOT NULL CHECK (octet_length(tx_hash) = 32),
  log_index    int         NOT NULL,
  name         text        NOT NULL,                -- ABI 事件名
  order_id     bigint,                              -- 订单类事件的链上订单号
  level        chain_level NOT NULL DEFAULT 'latest',
  topics       bytea[]     NOT NULL,                -- 原始 topics / data：订单状态由它们解码折叠
  data         bytea       NOT NULL,
  payload      jsonb       NOT NULL,                -- {"args": 按参数名展开（整数为十进制字符串）, "snapshot": 下单快照}
  created_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (chain_id, tx_hash, log_index)
);
CREATE INDEX escrow_events_block ON escrow_events USING brin (block_number);
-- 回滚、升级确认级别都只碰未最终确认的行。
CREATE INDEX escrow_events_unfinalized ON escrow_events (chain_id, block_number) WHERE level <> 'finalized';
CREATE INDEX escrow_events_order ON escrow_events (chain_id, contract, order_id, block_number, log_index)
  WHERE order_id IS NOT NULL;

-- 订单 = 该订单全部事件折叠的结果。回滚后事件全部撤销的订单保留行、state 置 0（链上 State.None），
-- 这样同一链上订单号重放后 id 不变，引用它的聊天等记录不受影响。
CREATE TABLE escrow_orders (
  id                bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  chain_id          bigint      NOT NULL REFERENCES chains (chain_id),
  contract          bytea       NOT NULL CHECK (octet_length(contract) = 20),
  chain_order_id    bigint      NOT NULL,
  mode              smallint    NOT NULL DEFAULT 0,  -- 0=一次性担保（Escrow）、1=分期（StreamEscrow，后续）
  buyer             bytea       NOT NULL CHECK (octet_length(buyer) = 20),
  seller            bytea       NOT NULL CHECK (octet_length(seller) = 20),
  buyer_id          bigint      REFERENCES users (id),   -- 钱包属于本站用户时
  seller_id         bigint      REFERENCES users (id),
  token             bytea       NOT NULL CHECK (octet_length(token) = 20),
  amount            numeric(38,0) NOT NULL,
  buyer_fee         numeric(38,0),                -- 以下费用来自下单区块的 getOrder 快照
  seller_fee        numeric(38,0),
  buyer_gas         numeric(38,0),
  seller_gas        numeric(38,0),
  dispute_fee       numeric(38,0),
  bond              numeric(38,0),
  instant           boolean     NOT NULL,
  offer_hash        bytea       NOT NULL,         -- 报价或拍卖授权的 EIP-712 哈希
  listing_hash      bytea       NOT NULL,
  auction_id        numeric(78,0),
  state             smallint    NOT NULL,         -- Escrow.State：0=无/已回滚 1=已付款 2=争议 3=已放款 4=已退款 5=已结算
  ship_deadline     timestamptz,
  confirm_deadline  timestamptz,                  -- 发货后才有
  proposer          bytea,
  proposal_to_buyer numeric(38,0),
  proposal_expiry   timestamptz,
  disputed_by       bytea,
  dispute_from_bond boolean,
  ruling_to_buyer_bps      int,
  ruling_bond_to_buyer_bps int,
  settled_to_buyer  numeric(38,0),
  settled_to_seller numeric(38,0),
  settled_to_fee    numeric(38,0),
  -- 分期订单（StreamEscrow）字段，一次性担保为空 / 0。
  interval_secs     int,
  periods           int,
  released_amount   numeric(38,0) NOT NULL DEFAULT 0,
  snapshot          jsonb,                        -- 下单区块 getOrder 的参数快照
  level             chain_level NOT NULL,         -- 全部事件中最低的确认级别
  created_block     bigint      NOT NULL,
  updated_block     bigint      NOT NULL,
  created_at        timestamptz NOT NULL DEFAULT now(),
  updated_at        timestamptz NOT NULL DEFAULT now(),
  UNIQUE (chain_id, contract, chain_order_id)
);
CREATE INDEX escrow_orders_buyer ON escrow_orders (buyer_id, id DESC) WHERE buyer_id IS NOT NULL;
CREATE INDEX escrow_orders_seller ON escrow_orders (seller_id, id DESC) WHERE seller_id IS NOT NULL;

-- 索引进度：已完整校对到的区块与当前 safe / finalized 区块号。
CREATE TABLE indexer_cursors (
  chain_id        bigint      PRIMARY KEY REFERENCES chains (chain_id),
  block_number    bigint      NOT NULL,
  block_hash      bytea       NOT NULL CHECK (octet_length(block_hash) = 32),
  safe_block      bigint      NOT NULL DEFAULT 0,
  finalized_block bigint      NOT NULL DEFAULT 0,
  updated_at      timestamptz NOT NULL DEFAULT now()
);

-- 近期游标历史（每轮校对一条），回滚时二分查找分叉点；最终确认之前的行定期清理。
CREATE TABLE indexer_checkpoints (
  chain_id     bigint NOT NULL REFERENCES chains (chain_id),
  block_number bigint NOT NULL,
  block_hash   bytea  NOT NULL CHECK (octet_length(block_hash) = 32),
  PRIMARY KEY (chain_id, block_number)
);
