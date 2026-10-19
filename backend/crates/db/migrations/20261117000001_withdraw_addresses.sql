-- 提币地址（plan 第 5 节「提币地址冷却」「提回交易所」）：第一个地址立即生效，之后新增的 24 小时后生效，期间可撤销。
-- forum-sponsor 只给转到本人已生效地址的 USDT `transfer` 代付。
--
-- 数据库账号（由 deploy/ansible 创建）另需：
--   GRANT SELECT, INSERT, UPDATE (revoked_at) ON withdraw_addresses TO forum_api;
--   GRANT SELECT ON withdraw_addresses TO forum_sponsor;

CREATE TABLE withdraw_addresses (
  id         bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  user_id    bigint      NOT NULL REFERENCES users (id),
  chain_id   bigint      NOT NULL,
  address    bytea       NOT NULL CHECK (octet_length(address) = 20),
  label      text        NOT NULL CHECK (char_length(label) <= 32),
  active_at  timestamptz NOT NULL,   -- 生效时间；冷却期内为 pending
  revoked_at timestamptz,            -- 撤销后不再生效，也不再占名额
  created_at timestamptz NOT NULL DEFAULT now()
);
-- 未撤销的地址同一用户同一条链不能重复（地址存 20 字节原值，大小写无关）；代付检查按它查。
CREATE UNIQUE INDEX withdraw_addresses_live ON withdraw_addresses (user_id, chain_id, address)
  WHERE revoked_at IS NULL;
-- 地址列表（含已撤销的）与「是否添加过地址」。
CREATE INDEX withdraw_addresses_user ON withdraw_addresses (user_id, id DESC);
