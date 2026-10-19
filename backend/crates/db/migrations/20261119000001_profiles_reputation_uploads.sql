-- 论坛缺口（plan 第 4、7、8、12 节）：交易板块、用户名与头像、账号交易信誉、经验流水、跨板块最新主题、图片上传。
--
-- forum_indexer 另需（deploy/ansible 授权）：信誉与交易经验只认最终确认的订单，由 indexer 在写订单的同一事务里更新。
--   GRANT SELECT, INSERT, UPDATE ON user_trade_stats, user_levels TO forum_indexer;
--   GRANT SELECT, INSERT ON exp_ledger TO forum_indexer;
--   GRANT SELECT ON level_thresholds TO forum_indexer;
-- forum_worker 每日全量重算信誉并修正偏差，按发帖发放经验：
--   GRANT SELECT, INSERT, UPDATE ON user_trade_stats, user_levels, job_cursors, uploads TO forum_worker;
--   GRANT SELECT, INSERT ON exp_ledger TO forum_worker;

-- 交易帖只能发在允许交易的板块。
ALTER TABLE forums ADD COLUMN allow_trade boolean NOT NULL DEFAULT false;

ALTER TABLE users
  ADD COLUMN username_changed_at timestamptz,   -- 用户名 30 天只能改一次；首次设置不受限
  ADD COLUMN avatar_url          text;          -- 处理完成的头像（本站图片地址，320 宽）

-- @提及前缀搜索（`~>=~` / `~<~` 区间，走 text_pattern_ops，与排序规则无关）。
CREATE INDEX users_username_prefix ON users (lower(username) text_pattern_ops) WHERE username IS NOT NULL;

-- 跨板块最新主题：重建 Valkey 有序集合与超出有序集合后的深分页（与 topics_forum_list 语义一致）。
CREATE INDEX topics_latest ON topics (last_post_at DESC, id DESC) WHERE (flags & 9) = 0;

-- 订单结果分类（plan 第 7 节「账号交易成功率」表），信誉统计与交易经验共用这一处定义：
--   1 = 成功（确认放款、确认期满放款、分期到期）       → 双方成功
--   2 = 卖家未按期发货 / 未开始服务即退款             → 卖家失败，买家不计入
--   3 = 仲裁判卖家责任（判给买家超过一半）            → 卖家失败 + 争议败诉，买家不计入
--   4 = 仲裁判买家责任（判给买家不到一半）            → 卖家成功，买家失败 + 争议败诉
--   5 = 协商和解、发货后卖家主动退款、各半裁决、仲裁超时兜底 → 不计入成功率，单独计次数
--   NULL = 未结束
-- 发货（分期为开始服务）之前卖家主动退款也按 2 计：对买家而言就是卖家没有履约。
-- 参数即 escrow_orders 的同名列；「已发货」对分期订单是已开始服务。
CREATE FUNCTION escrow_outcome(state smallint, mode smallint, confirm_deadline timestamptz,
                               started_at timestamptz, ruling_to_buyer_bps int)
RETURNS smallint LANGUAGE sql IMMUTABLE PARALLEL SAFE
RETURN CASE
  WHEN state = 3 THEN 1
  WHEN state = 4 AND (CASE mode WHEN 1 THEN started_at ELSE confirm_deadline END) IS NULL THEN 2
  WHEN state = 4 THEN 5
  WHEN state = 5 AND ruling_to_buyer_bps > 5000 THEN 3
  WHEN state = 5 AND ruling_to_buyer_bps < 5000 THEN 4
  WHEN state = 5 THEN 5
END;

-- 账号交易信誉：只由最终确认的订单结果得出（indexer 增量更新，worker 每日从 escrow_orders 全量重算校对）。
-- 防刷：只计 ≥ 10 USDT 的订单，同一对用户（不分买卖方向）每 30 天窗口只计 1 笔。
CREATE TABLE user_trade_stats (
  user_id           bigint        PRIMARY KEY REFERENCES users (id),
  success           int           NOT NULL DEFAULT 0,  -- 成功（卖家或买家身份）
  seller_fault_fail int           NOT NULL DEFAULT 0,  -- 作为卖家的责任失败
  buyer_fault_fail  int           NOT NULL DEFAULT 0,  -- 作为买家的责任失败
  disputes_lost     int           NOT NULL DEFAULT 0,
  settled           int           NOT NULL DEFAULT 0,  -- 和解等不计入成功率的结果
  volume            numeric(38,0) NOT NULL DEFAULT 0,  -- 成功订单金额合计（最小单位）
  avg_reply_secs    int,                               -- 卖家平均回复用时：私信功能上线后计算
  updated_at        timestamptz   NOT NULL DEFAULT now()
);

-- 等级所需经验（plan 第 7 节默认 6 级，后台可调 3–20 级）。等级不降：只在经验达到更高门槛时升级。
CREATE TABLE level_thresholds (
  level   smallint PRIMARY KEY CHECK (level BETWEEN 1 AND 20),
  min_exp bigint   NOT NULL UNIQUE CHECK (min_exp >= 0)
);
INSERT INTO level_thresholds (level, min_exp) VALUES (1, 0), (2, 100), (3, 500), (4, 2000), (5, 6000), (6, 15000);

-- 经验流水，只追加。reason：1=发主题、2=回复、3=精华（撤销记负数）、4=完成交易、5=争议败诉。
CREATE TABLE exp_ledger (
  id         bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  user_id    bigint      NOT NULL REFERENCES users (id),
  reason     smallint    NOT NULL,
  amount     int         NOT NULL,
  ref_id     bigint,                 -- 帖子 / 主题 / 订单 ID
  day        date        NOT NULL,   -- 来源事件的 UTC 日期：每日上限按它计
  dedupe     text,                   -- 同一来源只计一次（如同一对买卖双方 30 天一笔）
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX exp_ledger_dedupe ON exp_ledger (user_id, reason, dedupe) WHERE dedupe IS NOT NULL;
CREATE INDEX exp_ledger_user_day ON exp_ledger (user_id, reason, day);
CREATE INDEX exp_ledger_ref ON exp_ledger (user_id, reason, ref_id) WHERE ref_id IS NOT NULL;

CREATE FUNCTION exp_ledger_append_only() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'exp_ledger is append-only';
END;
$$;
CREATE TRIGGER exp_ledger_no_update_delete
  BEFORE UPDATE OR DELETE ON exp_ledger
  FOR EACH ROW EXECUTE FUNCTION exp_ledger_append_only();
CREATE TRIGGER exp_ledger_no_truncate
  BEFORE TRUNCATE ON exp_ledger
  FOR EACH STATEMENT EXECUTE FUNCTION exp_ledger_append_only();

-- worker 按主键顺序扫描的后台任务进度（如发帖经验：处理到哪个帖子 ID）。
CREATE TABLE job_cursors (
  name       text        PRIMARY KEY,
  position   bigint      NOT NULL DEFAULT 0,
  updated_at timestamptz NOT NULL DEFAULT now()
);

-- 图片上传（plan 第 12 节）：浏览器直传临时目录 → 确认 → worker 处理成 WebP → 正式目录。
CREATE TYPE upload_status AS ENUM ('pending', 'queued', 'processing', 'done', 'failed');
CREATE TABLE uploads (
  id           bigint        GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  user_id      bigint        NOT NULL REFERENCES users (id),
  status       upload_status NOT NULL DEFAULT 'pending',
  content_type text          NOT NULL,           -- 申请时声明的类型；处理时按内容重新判断
  size         int           NOT NULL CHECK (size BETWEEN 1 AND 10485760),
  tmp_key      text          NOT NULL UNIQUE,    -- 临时目录里的对象名
  width        int,                              -- 处理后（已按 EXIF 方向旋转）的原图尺寸
  height       int,
  variants     jsonb,                            -- [{"width", "height", "url"}]，从小到大
  error        text,                             -- 失败原因代码（unsupported / too_large / corrupt / missing）
  attempts     int           NOT NULL DEFAULT 0,
  lease_until  timestamptz,                      -- 处理中的租约：worker 崩溃后到期重新领取
  created_at   timestamptz   NOT NULL DEFAULT now(),
  updated_at   timestamptz   NOT NULL DEFAULT now()
);
CREATE INDEX uploads_user ON uploads (user_id, id DESC);
CREATE INDEX uploads_due ON uploads (id) WHERE status IN ('queued', 'processing');
