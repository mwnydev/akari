-- 私信、订单聊天、站内通知（plan 第 4、7、8 节）。
--
-- 数据库账号（由 deploy/ansible 创建）另需：
--   forum_indexer（报价付款卡片、订单系统消息、谈判记录关联）：
--     GRANT SELECT, INSERT ON messages, order_messages, order_negotiations TO forum_indexer;
--     GRANT SELECT, UPDATE (last_message_id, low_read_id, high_read_id) ON conversations TO forum_indexer;
--   forum_worker：GRANT SELECT, INSERT ON notifications; GRANT SELECT ON notification_reads, messages, conversations,
--     order_messages, escrow_orders TO forum_worker; 分区维护走 SECURITY DEFINER 函数 notifications_maintain。
--   所有账号都不授予 order_messages 的 UPDATE / DELETE / TRUNCATE（另有触发器兜底）。

-- ───────────────────────────── 私信 ─────────────────────────────

-- 一对一会话：两人只有一个会话（user_low < user_high）。各自的已读位置存在会话行上。
CREATE TABLE conversations (
  id              bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  user_low        bigint      NOT NULL REFERENCES users (id),
  user_high       bigint      NOT NULL REFERENCES users (id),
  low_read_id     bigint      NOT NULL DEFAULT 0,   -- user_low 已读到的消息 ID
  high_read_id    bigint      NOT NULL DEFAULT 0,
  last_message_id bigint,                           -- 还没有消息的会话不进会话列表
  created_at      timestamptz NOT NULL DEFAULT now(),
  CHECK (user_low < user_high),
  UNIQUE (user_low, user_high)
);
-- 会话列表：按两个身份各走一个索引再合并，游标 = 最后一条消息 ID（ID 递增即时间顺序）。
CREATE INDEX conversations_low_list ON conversations (user_low, last_message_id DESC)
  WHERE last_message_id IS NOT NULL;
CREATE INDEX conversations_high_list ON conversations (user_high, last_message_id DESC)
  WHERE last_message_id IS NOT NULL;

-- 私信消息。担保约定卡片是 kind = offer 的系统消息：body 为卡片事件，offer_id 指向报价。
CREATE TABLE messages (
  id              bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  conversation_id bigint      NOT NULL REFERENCES conversations (id),
  sender_id       bigint      REFERENCES users (id),   -- 买家付款等由系统写入的卡片为空
  kind            text        NOT NULL CHECK (kind IN ('text', 'image', 'offer')),
  body            text        NOT NULL DEFAULT '' CHECK (char_length(body) <= 2000),
  image           text,                                -- 图片上传上线后存图片键
  offer_id        bigint      REFERENCES offers (id),
  warn            boolean     NOT NULL DEFAULT false,  -- 含钱包地址或线下交易说法
  created_at      timestamptz NOT NULL DEFAULT now(),
  CHECK ((kind = 'offer') = (offer_id IS NOT NULL)),
  CHECK (kind <> 'image' OR image IS NOT NULL),
  CHECK (sender_id IS NOT NULL OR kind = 'offer')
);
CREATE INDEX messages_conversation ON messages (conversation_id, id);

-- 屏蔽：任一方屏蔽了对方，双方都不能再发私信（报价卡片等系统消息照常写入）。
CREATE TABLE user_blocks (
  blocker_id bigint      NOT NULL REFERENCES users (id),
  blocked_id bigint      NOT NULL REFERENCES users (id),
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (blocker_id, blocked_id),
  CHECK (blocker_id <> blocked_id)
);

-- 报价卡片关联会话。会话表建成前写入的私信报价（会话号没有意义）改挂到买卖双方的会话上；
-- 指向不存在会话的交易帖报价去掉会话号。
INSERT INTO conversations (user_low, user_high)
SELECT DISTINCT least(seller_id, buyer_id), greatest(seller_id, buyer_id)
FROM offers
WHERE conversation_id IS NOT NULL AND buyer_id IS NOT NULL AND buyer_id <> seller_id
ON CONFLICT (user_low, user_high) DO NOTHING;
UPDATE offers o SET conversation_id = c.id
FROM conversations c
WHERE o.conversation_id IS NOT NULL AND o.buyer_id IS NOT NULL AND o.buyer_id <> o.seller_id
  AND c.user_low = least(o.seller_id, o.buyer_id) AND c.user_high = greatest(o.seller_id, o.buyer_id);
UPDATE offers SET conversation_id = NULL
WHERE listing_id IS NOT NULL
  AND conversation_id IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM conversations c WHERE c.id = offers.conversation_id);
-- 表很小，直接加约束（不需要 NOT VALID + VALIDATE 分两步）。
ALTER TABLE offers
  ADD CONSTRAINT offers_conversation_fk FOREIGN KEY (conversation_id) REFERENCES conversations (id);

-- ───────────────────────────── 订单聊天 ─────────────────────────────

-- 订单内即时通讯（plan 第 7 节）：只追加，每条 hash = sha256(prev_hash ‖ 规范内容)，第一条 prev_hash 为 32 个 0 字节；
-- 规范内容见 forum-core `chat::canonical`。created_at 由写入方给定并参与哈希。
-- (order_id, prev_hash) 唯一：同一订单的链不能分叉（写入方另用事务级咨询锁串行化）。
CREATE TABLE order_messages (
  id         bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  order_id   bigint      NOT NULL REFERENCES escrow_orders (id),
  sender_id  bigint      REFERENCES users (id),   -- 系统消息（付款、发货、争议……）为空
  kind       text        NOT NULL CHECK (kind IN ('text', 'system', 'image')),
  body       text        NOT NULL CHECK (char_length(body) <= 2000),
  image      text,
  warn       boolean     NOT NULL DEFAULT false,
  prev_hash  bytea       NOT NULL CHECK (octet_length(prev_hash) = 32),
  hash       bytea       NOT NULL CHECK (octet_length(hash) = 32),
  created_at timestamptz NOT NULL,
  CHECK ((kind = 'system') = (sender_id IS NULL)),
  CHECK (kind <> 'image' OR image IS NOT NULL),
  UNIQUE (order_id, prev_hash)
);
CREATE INDEX order_messages_order ON order_messages (order_id, id);

CREATE FUNCTION order_messages_append_only() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'order_messages is append-only';
END;
$$;
CREATE TRIGGER order_messages_no_update_delete
  BEFORE UPDATE OR DELETE ON order_messages
  FOR EACH ROW EXECUTE FUNCTION order_messages_append_only();
CREATE TRIGGER order_messages_no_truncate
  BEFORE TRUNCATE ON order_messages
  FOR EACH STATEMENT EXECUTE FUNCTION order_messages_append_only();
REVOKE UPDATE, DELETE, TRUNCATE ON order_messages FROM PUBLIC;

-- 订单聊天已读位置（买卖双方与仲裁员各一行）。
CREATE TABLE order_message_reads (
  order_id     bigint      NOT NULL REFERENCES escrow_orders (id),
  user_id      bigint      NOT NULL REFERENCES users (id),
  last_read_id bigint      NOT NULL,
  updated_at   timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (order_id, user_id)
);

-- 建单前的私信谈判记录（仲裁证据，plan 第 7 节「仲裁规则」）：报价卡片生成的订单在付款上链时记下会话与当时最后一条消息。
-- 区块回滚后同一订单重放时保留第一次的记录（订单 id 不变）。
CREATE TABLE order_negotiations (
  order_id        bigint      PRIMARY KEY REFERENCES escrow_orders (id),
  conversation_id bigint      NOT NULL REFERENCES conversations (id),
  offer_id        bigint      NOT NULL REFERENCES offers (id),
  last_message_id bigint      NOT NULL,
  created_at      timestamptz NOT NULL DEFAULT now()
);

-- ───────────────────────────── 站内通知 ─────────────────────────────

-- 按月分区（plan 第 8 节），保留 180 天：worker 每天调用 notifications_maintain 建好未来两个月的分区、删掉过期分区。
-- 已读用每人一个水位（notification_reads），通知行本身只追加不更新。
-- 列表按 id 游标（id 递增即时间顺序）：每个分区上的 (user_id, id DESC) 索引，Merge Append 取前 N 条即停。
CREATE TABLE notifications (
  id         bigint      GENERATED ALWAYS AS IDENTITY,
  user_id    bigint      NOT NULL,
  kind       text        NOT NULL CHECK (kind IN ('reply', 'mention', 'message', 'order', 'offer', 'system')),
  actor_id   bigint,
  payload    jsonb       NOT NULL DEFAULT '{}',
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (id, created_at)
) PARTITION BY RANGE (created_at);
CREATE INDEX notifications_user ON notifications (user_id, id DESC);

CREATE TABLE notification_reads (
  user_id      bigint      PRIMARY KEY REFERENCES users (id),
  last_read_id bigint      NOT NULL,
  updated_at   timestamptz NOT NULL DEFAULT now()
);

-- 建好 `ref_time` 所在月及之后 `ahead` 个月的分区（UTC 月份，表名 notifications_YYYYMM），
-- 删掉整月都早于 `ref_time - keep` 的分区。
-- 返回新建与删除的分区名。
CREATE FUNCTION notifications_maintain(
  ref_time timestamptz DEFAULT now(), keep interval DEFAULT '180 days', ahead int DEFAULT 2
) RETURNS TABLE (created text[], dropped text[])
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public AS $$
DECLARE
  month timestamp;
  part text;
  upper_bound timestamp;
BEGIN
  created := '{}';
  dropped := '{}';
  FOR i IN 0..ahead LOOP
    month := date_trunc('month', ref_time AT TIME ZONE 'UTC') + make_interval(months => i);
    part := 'notifications_' || to_char(month, 'YYYYMM');
    IF to_regclass(part) IS NULL THEN
      EXECUTE format(
        'CREATE TABLE %I PARTITION OF notifications FOR VALUES FROM (%L) TO (%L)',
        part, month::text || '+00', (month + interval '1 month')::text || '+00');
      created := created || part;
    END IF;
  END LOOP;
  FOR part IN
    SELECT c.relname FROM pg_inherits i JOIN pg_class c ON c.oid = i.inhrelid
    WHERE i.inhparent = 'notifications'::regclass AND c.relname ~ '^notifications_[0-9]{6}$'
    ORDER BY c.relname
  LOOP
    upper_bound := to_date(right(part, 6), 'YYYYMM')::timestamp + interval '1 month';
    IF upper_bound <= (ref_time - keep) AT TIME ZONE 'UTC' THEN
      EXECUTE format('DROP TABLE %I', part);
      dropped := dropped || part;
    END IF;
  END LOOP;
  RETURN NEXT;
END;
$$;
SELECT notifications_maintain();
