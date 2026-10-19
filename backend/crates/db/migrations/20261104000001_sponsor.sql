-- forum-sponsor（plan 第 5 节「gas 规则」）：代付签名记录、每单代付次数、链上失败暂停。只有 forum-sponsor 写入。
--
-- 数据库账号（由 deploy/ansible 创建）应授予：
--   GRANT SELECT, INSERT, UPDATE ON sponsorships, sponsored_calls, sponsor_accounts TO forum_sponsor;
--   GRANT SELECT ON wallets TO forum_sponsor;            -- 只给本人钱包代付
--   GRANT SELECT, INSERT ON outbox TO forum_sponsor;     -- 押金、热钱包余额告警（去重要读）
--   GRANT USAGE ON ALL SEQUENCES IN SCHEMA public TO forum_sponsor;

-- signed：已签代付，还没收到 eth_sendUserOperation；replaced：同一 nonce 重新签过（旧签名仍可能被别处提交上链）；
-- submitted：已进代发队列或转交备用 bundler；included / failed：已上链，callData 成功 / 回滚（失败也已收费、耗 gas）；
-- expired：签名过期且没有上链；dropped：代发前链上验证不通过（未上链、未收费）。
CREATE TYPE sponsorship_status AS ENUM ('signed', 'replaced', 'submitted', 'included', 'failed', 'expired', 'dropped');

CREATE TABLE sponsorships (
  id              bigint        GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  user_id         bigint        NOT NULL REFERENCES users (id),
  chain_id        bigint        NOT NULL,
  sender          bytea         NOT NULL CHECK (octet_length(sender) = 20),
  nonce           numeric(78,0) NOT NULL,
  user_op_hash    bytea         NOT NULL UNIQUE CHECK (octet_length(user_op_hash) = 32),
  fee             numeric(38,0) NOT NULL CHECK (fee >= 0),   -- USDT 最小单位
  valid_until     timestamptz   NOT NULL,
  signed_block    bigint        NOT NULL,                    -- 签名时的最新区块：过期对账从这里查日志
  status          sponsorship_status NOT NULL DEFAULT 'signed',
  user_op         jsonb,                                     -- 提交时的完整 UserOperation（eth_getUserOperationByHash）
  via_fallback    boolean       NOT NULL DEFAULT false,      -- 经第三方 bundler 提交
  tx_hash         bytea         CHECK (octet_length(tx_hash) = 32),
  block_number    bigint,
  actual_gas_cost numeric(78,0),                             -- wei，EntryPoint 从押金扣的实际金额
  created_at      timestamptz   NOT NULL DEFAULT now(),
  updated_at      timestamptz   NOT NULL DEFAULT now()
);
-- 「每个账户同一时间只有一笔待上链」与过期对账只看未结束的行。
CREATE INDEX sponsorships_open ON sponsorships (sender, valid_until)
  WHERE status IN ('signed', 'replaced', 'submitted');
CREATE INDEX sponsorships_open_until ON sponsorships (valid_until)
  WHERE status IN ('signed', 'replaced', 'submitted');
-- 链上失败计数。
CREATE INDEX sponsorships_user_failed ON sponsorships (user_id, updated_at) WHERE status = 'failed';

-- 一笔操作里的担保合约调用，用来数每单的代付次数（标记发货 1 次、领取每天 1 次……）。
CREATE TABLE sponsored_calls (
  sponsorship_id bigint        NOT NULL REFERENCES sponsorships (id),
  contract       bytea         NOT NULL CHECK (octet_length(contract) = 20),
  order_id       numeric(78,0) NOT NULL,
  method         text          NOT NULL,
  free           boolean       NOT NULL,     -- 走免费额度（否则这次收了代付费）
  PRIMARY KEY (sponsorship_id, contract, order_id, method)
);
CREATE INDEX sponsored_calls_order ON sponsored_calls (contract, order_id, method);

-- 链上失败 3 次暂停代付 24 小时（plan 第 9 节）。窗口从第一次失败算起，暂停后重新计数。
CREATE TABLE sponsor_accounts (
  user_id          bigint      PRIMARY KEY REFERENCES users (id),
  failures         int         NOT NULL DEFAULT 0,
  first_failure_at timestamptz,
  suspended_until  timestamptz,
  updated_at       timestamptz NOT NULL DEFAULT now()
);

-- 余额告警去重：同一种告警一段时间内只写一条。
CREATE INDEX outbox_alerts ON outbox (kind, created_at) WHERE kind LIKE 'alert.%';
