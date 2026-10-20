-- 拍卖（plan 第 6、7、9 节「拍卖」，ADR 0011）：出价全在论坛里进行、只追加；结束后 forum-sponsor 签成交证明，
-- 赢家用卖家的拍卖授权 + 成交证明调用 `Escrow.createFromAuction` 付款建单，之后与一次性担保完全相同。
--
-- 数据库账号（deploy/ansible 授权）：
--   forum_api：GRANT SELECT, INSERT, UPDATE ON auctions; GRANT SELECT, INSERT ON auction_bids;
--     GRANT SELECT ON auction_proofs, auction_passes, auction_defaults TO forum_api;
--   forum_worker：GRANT SELECT, UPDATE (state, round, pay_deadline, updated_at) ON auctions;
--     GRANT SELECT ON auction_bids, auction_proofs; GRANT SELECT, INSERT ON auction_passes, auction_defaults;
--     GRANT INSERT ON sanctions, admin_audit_logs TO forum_worker;   -- 违约自动禁拍
--   forum_sponsor：GRANT SELECT ON auctions, auction_bids, auction_passes, auction_defaults, site_settings;
--     GRANT SELECT, INSERT ON auction_proofs TO forum_sponsor;      -- 成交证明只有 forum-sponsor 写
--   forum_indexer：GRANT SELECT, UPDATE (state, order_id, updated_at) ON auctions TO forum_indexer;   -- 链上成交 / 回滚
--   REVOKE INSERT, UPDATE, DELETE ON auction_proofs FROM forum_api, forum_worker, forum_indexer;

-- 状态：0 草稿（卖家还没签授权）、1 进行中、2 待付款（已发成交证明）、3 已成交（indexer 见到 AuctionSettled）、
-- 4 流拍（未达保留价、无人出价或递补用完）、5 已取消（卖家在第一口出价前取消）。
-- 结束时间到了但 worker 还没处理的进行中拍卖，接口按「已结束」展示。
CREATE TABLE auctions (
  id                   bigint        GENERATED ALWAYS AS IDENTITY PRIMARY KEY,   -- 也是 AuctionAuth.auctionId
  seller_id            bigint        NOT NULL REFERENCES users (id),
  forum_id             int           NOT NULL REFERENCES forums (id),
  topic_id             bigint        UNIQUE REFERENCES topics (id),              -- 上线（签名验证通过）时建主题
  chain_id             bigint        NOT NULL,
  verifying_contract   bytea         NOT NULL CHECK (octet_length(verifying_contract) = 20),
  seller               bytea         NOT NULL CHECK (octet_length(seller) = 20), -- 卖家钱包（授权的 seller）
  -- 金额都是拍卖币种的最小单位：USDT 最小单位或分。
  currency             smallint      NOT NULL CHECK (currency IN (0, 1)),
  cny_per_usdt         int           NOT NULL DEFAULT 0,
  start_price          numeric(38,0) NOT NULL CHECK (start_price > 0),
  increment            numeric(38,0) NOT NULL CHECK (increment > 0),
  reserve_price        numeric(38,0) NOT NULL CHECK (reserve_price >= start_price),
  duration_secs        int           NOT NULL CHECK (duration_secs > 0),
  -- 出价资格：0 不要求、1 余额 ≥ 出价的 eligibility_value%、2 余额 ≥ eligibility_value（USDT 最小单位）。
  eligibility          smallint      NOT NULL DEFAULT 0 CHECK (eligibility IN (0, 1, 2)),
  eligibility_value    numeric(38,0),
  terms                jsonb         NOT NULL,   -- 成交后的担保条款：instant、ship_window、confirm_window、fee_payer
  content_hash         bytea         NOT NULL CHECK (octet_length(content_hash) = 32),   -- AuctionAuth.listingHash
  draft                jsonb,                    -- 上线前的帖子内容（标题、过滤后的 HTML、纯文本、原稿、@）；上线后清空
  -- 签名：开始签名时按当时的参数生成授权，签名时限内提交、验证通过后上线。
  auth                 jsonb,                    -- AuctionAuth（typed data 的 message）
  auth_hash            bytea         UNIQUE CHECK (octet_length(auth_hash) = 32),
  sign_deadline        timestamptz,
  seller_signature     bytea,
  -- 防狙击参数在开始签名时定下（授权有效期按它们算），之后站长改参数不影响这场拍卖。
  snipe_window_secs    int,
  snipe_extension_secs int,
  extension_cap_secs   int,
  state                smallint      NOT NULL DEFAULT 0 CHECK (state BETWEEN 0 AND 5),
  starts_at            timestamptz,
  ends_at              timestamptz,
  max_ends_at          timestamptz,              -- ends_at 的顺延上限
  top_amount           numeric(38,0),
  top_bidder_id        bigint        REFERENCES users (id),
  bid_count            int           NOT NULL DEFAULT 0,
  round                smallint      NOT NULL DEFAULT 0 CHECK (round BETWEEN 0 AND 3),   -- 已发出的成交证明份数
  pay_deadline         timestamptz,              -- 当前一份成交证明的付款截止时间
  order_id             bigint        REFERENCES escrow_orders (id),
  created_at           timestamptz   NOT NULL DEFAULT now(),
  updated_at           timestamptz   NOT NULL DEFAULT now(),
  CHECK ((currency = 0 AND cny_per_usdt = 0) OR (currency = 1 AND cny_per_usdt BETWEEN 50000 AND 100000)),
  CHECK ((eligibility = 0) = (eligibility_value IS NULL)),
  CHECK (state = 0 OR (topic_id IS NOT NULL AND seller_signature IS NOT NULL AND ends_at IS NOT NULL) OR state = 5),
  CHECK ((state = 3) = (order_id IS NOT NULL))
);
-- worker：结束时间已到的进行中拍卖、付款期已过的待付款拍卖。
CREATE INDEX auctions_live_end ON auctions (ends_at) WHERE state = 1;
CREATE INDEX auctions_pay_due ON auctions (pay_deadline) WHERE state = 2;
CREATE INDEX auctions_seller ON auctions (seller_id, id DESC);

-- 出价：只追加（应用账号没有改删权限）。amount 为拍卖币种，amount_usdt 为按卖家汇率折合的 USDT（向上取整）。
CREATE TABLE auction_bids (
  id          bigint        GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  auction_id  bigint        NOT NULL REFERENCES auctions (id),
  bidder_id   bigint        NOT NULL REFERENCES users (id),
  amount      numeric(38,0) NOT NULL CHECK (amount > 0),
  amount_usdt numeric(38,0) NOT NULL CHECK (amount_usdt > 0),
  linked      boolean       NOT NULL DEFAULT false,   -- 与本场其他出价人是关联账号（同设备）：公开出价记录里提示
  created_at  timestamptz   NOT NULL DEFAULT now()
);
-- 出价记录翻页（plan 第 8 节：auction_bids 索引 (auction_id, id DESC)）。
CREATE INDEX auction_bids_auction ON auction_bids (auction_id, id DESC);
-- 结算：本场出价从高到低（同价先出的在前），找赢家与递补者。
CREATE INDEX auction_bids_rank ON auction_bids (auction_id, amount DESC, id);
-- 某人在本场的出价（我的最高价、关联账号标记）。
CREATE INDEX auction_bids_bidder ON auction_bids (auction_id, bidder_id, amount DESC);
REVOKE UPDATE, DELETE, TRUNCATE ON auction_bids FROM PUBLIC;

-- forum-sponsor 签发的成交证明：每场每轮最多一份（主键），最多 3 轮。只有 forum-sponsor 写入；代付
-- `createFromAuction` 时核对调用里的证明就是这里最新一轮的那份，且付款人是证明里的赢家。
CREATE TABLE auction_proofs (
  auction_id   bigint        NOT NULL REFERENCES auctions (id),
  round        smallint      NOT NULL CHECK (round BETWEEN 1 AND 3),
  bid_id       bigint        NOT NULL REFERENCES auction_bids (id),
  bidder_id    bigint        NOT NULL REFERENCES users (id),
  auth_hash    bytea         NOT NULL CHECK (octet_length(auth_hash) = 32),
  winner       bytea         NOT NULL CHECK (octet_length(winner) = 20),
  price        numeric(38,0) NOT NULL,
  pay_deadline timestamptz   NOT NULL,
  signature    bytea         NOT NULL,
  created_at   timestamptz   NOT NULL DEFAULT now(),
  PRIMARY KEY (auction_id, round),
  UNIQUE (auction_id, bidder_id)
);
CREATE INDEX auction_proofs_auth ON auction_proofs (auth_hash, round DESC);
REVOKE UPDATE, DELETE, TRUNCATE ON auction_proofs FROM PUBLIC;

-- 结束时（或递补时）不再符合出价资格、被跳过的出价人（被封号 / 禁拍、余额不足、没有钱包）。
CREATE TABLE auction_passes (
  auction_id bigint      NOT NULL REFERENCES auctions (id),
  bidder_id  bigint      NOT NULL REFERENCES users (id),
  reason     text        NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (auction_id, bidder_id)
);

-- 赢家违约：付款期内没有成交。记一次违约并自动禁拍（sanction_id），只追加。
CREATE TABLE auction_defaults (
  id           bigint        GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  auction_id   bigint        NOT NULL REFERENCES auctions (id),
  round        smallint      NOT NULL,
  user_id      bigint        NOT NULL REFERENCES users (id),
  price        numeric(38,0) NOT NULL,
  pay_deadline timestamptz   NOT NULL,
  sanction_id  bigint        REFERENCES sanctions (id),
  created_at   timestamptz   NOT NULL DEFAULT now(),
  UNIQUE (auction_id, round)
);
CREATE INDEX auction_defaults_user ON auction_defaults (user_id, id DESC);
REVOKE UPDATE, DELETE, TRUNCATE ON auction_defaults FROM PUBLIC;

-- 关联账号判断按设备查另一个用户。
CREATE INDEX user_devices_device ON user_devices (device_hash);

-- 系统自动的处罚（拍卖违约禁拍）没有操作人。
ALTER TABLE sanctions ALTER COLUMN created_by DROP NOT NULL;

-- 两个账号是否关联：登录过同一台设备（plan 第 9 节「关联账号出价标记」）。
CREATE FUNCTION users_linked(p_a bigint, p_b bigint) RETURNS boolean
LANGUAGE sql STABLE PARALLEL SAFE
RETURN p_a <> p_b AND EXISTS (
  SELECT 1 FROM user_devices a JOIN user_devices b ON b.device_hash = a.device_hash
  WHERE a.user_id = p_a AND b.user_id = p_b);

-- 拍卖站内通知（被超价、中标、递补、违约禁拍、成交、流拍）：发件箱 notify.auction 投递成 kind = 'auction'。
-- 约束里没有的类型会让整批投递失败，所以与新类型同一迁移放开；已有通知的类型都在列表里，重建不影响旧行。
ALTER TABLE notifications DROP CONSTRAINT notifications_kind_check;
ALTER TABLE notifications ADD CONSTRAINT notifications_kind_check
  CHECK (kind IN ('reply', 'mention', 'message', 'order', 'offer', 'system', 'like', 'comment', 'auction'));
