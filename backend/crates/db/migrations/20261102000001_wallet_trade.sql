-- 开通交易（plan 第 5 节）：钱包 Passkey 公钥、智能账户地址；交易帖与卖家报价（第 6、7 节）。

-- 钱包 Passkey 只接受 ES256（P-256）：链上 Kernel WebAuthn 验证器只认 P-256 公钥坐标。
ALTER TABLE passkeys
  ADD COLUMN public_key_cose bytea,          -- COSE_Key（EC2 / P-256 / ES256），CBOR 编码
  ADD COLUMN pubkey_x        bytea CHECK (octet_length(pubkey_x) = 32),
  ADD COLUMN pubkey_y        bytea CHECK (octet_length(pubkey_y) = 32),
  ADD COLUMN last_used_at    timestamptz,
  ADD CONSTRAINT passkeys_wallet_key CHECK (
    purpose <> 1 OR (public_key_cose IS NOT NULL AND pubkey_x IS NOT NULL AND pubkey_y IS NOT NULL)
  );

-- 钱包地址由浏览器用 Passkey 公钥计算（Kernel v3.3 counterfactual 地址），绑定到计算时用的那个 Passkey。
ALTER TABLE wallets
  ADD COLUMN passkey_id bigint NOT NULL REFERENCES passkeys (id),
  ADD COLUMN account_kind smallint NOT NULL DEFAULT 1;  -- 1 = Kernel v3.3 + WebAuthn 验证器（ADR 0003）

-- 交易帖：主题 kind = 1。价格与条款是发帖时卖家填写的预填值，实际成交以签名报价为准。
CREATE TABLE trade_listings (
  id           bigint        GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  topic_id     bigint        NOT NULL UNIQUE REFERENCES topics (id),
  seller_id    bigint        NOT NULL REFERENCES users (id),
  chain_id     bigint        NOT NULL,
  price        numeric(38,0) NOT NULL CHECK (price > 0),   -- USDT 最小单位
  terms        jsonb         NOT NULL,
  content_hash bytea         NOT NULL CHECK (octet_length(content_hash) = 32),  -- 报价里的 listingHash
  created_at   timestamptz   NOT NULL DEFAULT now()
);
CREATE INDEX trade_listings_seller ON trade_listings (seller_id, id DESC);

-- 报价卡片状态（plan 第 7 节「担保约定卡片」）。paid 由 indexer 看到建单事件后写入。
CREATE TYPE offer_status AS ENUM ('pending', 'modified', 'rejected', 'cancelled', 'expired', 'paid');

-- 卖家签名的报价。EIP-712 字段逐一成列（与 Escrow.Offer 同名同序），付款时原样还原 typed data。
CREATE TABLE offers (
  id                 bigint        GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  listing_id         bigint        REFERENCES trade_listings (id),
  conversation_id    bigint,                                -- 私信会话；会话表在私信功能里补
  seller_id          bigint        NOT NULL REFERENCES users (id),
  buyer_id           bigint        REFERENCES users (id),   -- 公开报价为空
  replaces_id        bigint        REFERENCES offers (id),  -- 修改约定时被替换的旧卡片
  chain_id           bigint        NOT NULL,
  verifying_contract bytea         NOT NULL CHECK (octet_length(verifying_contract) = 20),
  seller             bytea         NOT NULL CHECK (octet_length(seller) = 20),
  buyer              bytea         NOT NULL CHECK (octet_length(buyer) = 20),  -- 全 0 = 公开报价
  token              bytea         NOT NULL CHECK (octet_length(token) = 20),
  amount             numeric(38,0) NOT NULL,
  fee_payer          smallint      NOT NULL CHECK (fee_payer BETWEEN 0 AND 2),
  max_fee_bps        int           NOT NULL CHECK (max_fee_bps BETWEEN 0 AND 65535),
  max_base_gas       numeric(38,0) NOT NULL,
  max_min_fee        numeric(38,0) NOT NULL,
  bond               numeric(38,0) NOT NULL,
  late_slash_bps     int           NOT NULL CHECK (late_slash_bps BETWEEN 0 AND 65535),
  instant            boolean       NOT NULL,
  ship_window        bigint        NOT NULL CHECK (ship_window BETWEEN 0 AND 4294967295),
  confirm_window     bigint        NOT NULL CHECK (confirm_window BETWEEN 0 AND 4294967295),
  listing_hash       bytea         NOT NULL CHECK (octet_length(listing_hash) = 32),
  expiry             bigint        NOT NULL CHECK (expiry >= 0),
  max_fills          bigint        NOT NULL CHECK (max_fills BETWEEN 1 AND 4294967295),
  nonce              numeric(78,0) NOT NULL,                -- uint256 最多 78 位十进制
  signature          bytea         NOT NULL,
  offer_hash         bytea         NOT NULL UNIQUE CHECK (octet_length(offer_hash) = 32),
  status             offer_status  NOT NULL DEFAULT 'pending',
  reject_reason      text,
  expires_at         timestamptz   NOT NULL,
  created_at         timestamptz   NOT NULL DEFAULT now(),
  updated_at         timestamptz   NOT NULL DEFAULT now(),
  CHECK (listing_id IS NOT NULL OR conversation_id IS NOT NULL)
);
CREATE INDEX offers_listing ON offers (listing_id, id DESC) WHERE listing_id IS NOT NULL;
CREATE INDEX offers_conversation ON offers (conversation_id, id DESC) WHERE conversation_id IS NOT NULL;
CREATE INDEX offers_seller ON offers (seller_id, id DESC);
CREATE INDEX offers_buyer ON offers (buyer_id, id DESC) WHERE buyer_id IS NOT NULL;
-- worker 过期任务只扫待确认卡片。
CREATE INDEX offers_pending_expiry ON offers (expires_at) WHERE status = 'pending';
