CREATE TABLE users (
  id               bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  email            text        NOT NULL,              -- 归一化后（去首尾空白、小写）
  username         text,                              -- 注册后由用户设置；设置前为空
  show_online      boolean     NOT NULL DEFAULT true,
  last_seen_at     timestamptz,
  created_at       timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT users_email_normalized CHECK (email = lower(btrim(email)))
);
CREATE UNIQUE INDEX users_email ON users (email);
CREATE UNIQUE INDEX users_username ON users (lower(username)) WHERE username IS NOT NULL;

-- 登录过的设备：设备 ID 存在签名 Cookie 里，这里只存其 SHA-256，用于「新设备登录」邮件提醒。
CREATE TABLE user_devices (
  user_id       bigint      NOT NULL REFERENCES users (id),
  device_hash   bytea       NOT NULL,
  first_seen_at timestamptz NOT NULL DEFAULT now(),
  last_seen_at  timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, device_hash)
);

-- 钱包 Passkey 与管理人员 Passkey（plan 第 5 节）；结构在开通交易阶段补全。
CREATE TABLE passkeys (
  id            bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  user_id       bigint      NOT NULL REFERENCES users (id),
  purpose       smallint    NOT NULL,                -- 1=钱包签名、2=后台登录
  credential_id bytea       NOT NULL UNIQUE,
  credential    jsonb       NOT NULL,                -- webauthn-rs 序列化的凭证
  created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX passkeys_user ON passkeys (user_id);

CREATE TABLE wallets (
  id         bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  user_id    bigint      NOT NULL REFERENCES users (id),
  chain_id   bigint      NOT NULL,
  address    bytea       NOT NULL CHECK (octet_length(address) = 20),
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (chain_id, address),
  UNIQUE (user_id, chain_id)
);
