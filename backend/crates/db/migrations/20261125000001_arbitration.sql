-- 仲裁（plan 第 7 节「仲裁规则」、ADR 0013 第 2、4、5 条）：案件、分配与回避、裁决、当事人陈述、裁决 Passkey。
--
-- 案件由 forum-indexer 在订单进入争议时创建（同一事务），订单离开争议状态时关闭、区块回滚回到争议时重新打开；
-- 分配、超时改派、申诉期到期由 forum-worker 处理；裁决、申诉、叫停由 forum-api 写入。所有状态变化写只增审计日志
-- （admin_audit_logs，系统操作 actor_id = 0）。
--
-- 数据库账号（由 deploy/ansible 创建）另需：
--   forum_indexer：GRANT SELECT, INSERT, UPDATE ON arbitration_cases TO forum_indexer;
--     GRANT SELECT ON arbitration_rulings, staff_roles, users TO forum_indexer;   -- 执行结果与已公示裁决比对、提醒站长
--     GRANT INSERT ON admin_audit_logs TO forum_indexer;
--   forum_worker：GRANT SELECT, UPDATE ON arbitration_cases; GRANT SELECT, INSERT, UPDATE ON arbitration_assignments;
--     GRANT SELECT ON arbitration_rulings, staff_roles; GRANT INSERT ON admin_audit_logs TO forum_worker;
--   forum_api：GRANT SELECT, UPDATE ON arbitration_cases; GRANT SELECT, UPDATE ON arbitration_assignments;
--     GRANT SELECT, INSERT ON arbitration_rulings, arbitration_statements TO forum_api;
--   所有账号都不授予 arbitration_rulings / arbitration_statements 的 UPDATE / DELETE / TRUNCATE（另有触发器兜底）。

-- passkeys.purpose 新增 3 = 论坛裁决 Passkey（RP ID `akari.cc`，只用于签署裁决；与钱包 Passkey 互不通用）。
COMMENT ON COLUMN passkeys.purpose IS '1=钱包签名（RP 钱包中心域名）、2=后台登录、3=论坛裁决签署（RP 论坛主域名）';

-- status（流程状态）：
--   reviewing      待仲裁员裁决（< 200 USDT 一人，≥ 200 USDT 两人独立裁决）
--   owner_review   两人意见不一致，待站长终裁
--   appeal_window  裁决已公示，72 小时申诉期
--   appealed       已申诉，待未参与的仲裁员（或站长）复核
--   ready          可由仲裁 Safe 执行 resolve
--   halted         站长在执行前叫停
-- closed_at / close_reason 由 indexer 按订单链上状态写入，与流程状态分开：区块回滚使订单回到争议时清空，流程照旧继续。
--   executed = Resolved 已上链；settled = 和解或超时兜底等其他方式结束；reverted = 争议交易被区块回滚撤销。
CREATE TABLE arbitration_cases (
  id                 bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  order_id           bigint      NOT NULL UNIQUE REFERENCES escrow_orders (id),
  status             text        NOT NULL DEFAULT 'reviewing'
                     CHECK (status IN ('reviewing', 'owner_review', 'appeal_window', 'appealed', 'ready', 'halted')),
  panel_size         smallint    NOT NULL CHECK (panel_size IN (1, 2)),
  evidence_until     timestamptz NOT NULL,          -- 举证期截止（争议发起后 72 小时）
  final_ruling_id    bigint,                        -- 当前生效（已公示）的裁决
  published_at       timestamptz,
  appeal_until       timestamptz,
  appeal_by          bigint      REFERENCES users (id),
  appeal_reason      text        CHECK (char_length(appeal_reason) <= 2000),
  appealed_at        timestamptz,
  halted_from        text,                          -- 叫停前的状态，恢复时回到它
  halted_by          bigint      REFERENCES users (id),
  halt_reason        text        CHECK (char_length(halt_reason) <= 2000),
  ready_at           timestamptz,
  closed_at          timestamptz,
  close_reason       text        CHECK (close_reason IN ('executed', 'settled', 'reverted')),
  stalled_alerted_at timestamptz,                   -- 找不到可分配的仲裁员时已提醒站长（分配成功后清空）
  timeout_alerted_at timestamptz,                   -- resolveTimeout 不足 3 天时已提醒
  created_at         timestamptz NOT NULL DEFAULT now(),
  updated_at         timestamptz NOT NULL DEFAULT now(),
  CHECK ((closed_at IS NULL) = (close_reason IS NULL)),
  CHECK (status <> 'halted' OR halted_from IS NOT NULL)
);
-- worker 与队列只扫未关闭的案件。
CREATE INDEX arbitration_cases_open ON arbitration_cases (status, id) WHERE closed_at IS NULL;

-- 分配：role = panel（裁决）或 appeal（申诉复核）。同一人在同一案件只能被分配一次：被改派走的不再分回，
-- 复核人必然是「未参与」的人。ended_reason：ruled = 已提交裁决，reassigned = 超时改派。
CREATE TABLE arbitration_assignments (
  id             bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  case_id        bigint      NOT NULL REFERENCES arbitration_cases (id),
  arbitrator_id  bigint      NOT NULL REFERENCES users (id),
  role           text        NOT NULL CHECK (role IN ('panel', 'appeal')),
  assigned_at    timestamptz NOT NULL DEFAULT now(),
  due_at         timestamptz NOT NULL,
  ended_at       timestamptz,
  ended_reason   text        CHECK (ended_reason IN ('ruled', 'reassigned')),
  CHECK ((ended_at IS NULL) = (ended_reason IS NULL)),
  UNIQUE (case_id, arbitrator_id)
);
CREATE INDEX arbitration_assignments_open ON arbitration_assignments (arbitrator_id, due_at) WHERE ended_at IS NULL;
-- 工作量与按时率统计（只按分配与按时提交计酬，不含裁决方向）。
CREATE INDEX arbitration_assignments_stats ON arbitration_assignments (arbitrator_id, assigned_at);

-- 裁决（只追加）。kind：panel = 仲裁员、owner = 站长终裁（意见不一致或叫停后改判）、appeal = 申诉复核。
-- signed_intent 是裁决 Passkey 签署的规范内容（challenge = sha256("akari-ruling\0" ‖ signed_intent)），
-- assertion 是浏览器返回的完整断言（clientDataJSON、authenticatorData、签名），事后可用该 Passkey 公钥独立复核。
CREATE TABLE arbitration_rulings (
  id                  bigint        GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  case_id             bigint        NOT NULL REFERENCES arbitration_cases (id),
  arbitrator_id       bigint        NOT NULL REFERENCES users (id),
  assignment_id       bigint        REFERENCES arbitration_assignments (id),  -- 站长终裁 / 站长复核为空
  kind                text          NOT NULL CHECK (kind IN ('panel', 'owner', 'appeal')),
  to_buyer_bps        int           NOT NULL CHECK (to_buyer_bps BETWEEN 0 AND 10000),
  bond_to_buyer_bps   int           NOT NULL CHECK (bond_to_buyer_bps BETWEEN 0 AND 10000),
  principal_to_buyer  numeric(38,0) NOT NULL,       -- 按裁决时的链上订单算出的本金分配（未计手续费与争议费）
  principal_to_seller numeric(38,0) NOT NULL,
  bond_to_buyer       numeric(38,0) NOT NULL,
  reason              text          NOT NULL CHECK (char_length(reason) BETWEEN 1 AND 2000),  -- 公开理由
  chat_head           bytea         NOT NULL CHECK (octet_length(chat_head) = 32),            -- 裁决时校验通过的聊天链头
  chat_count          bigint        NOT NULL,
  signed_intent       text          NOT NULL,
  challenge           bytea         NOT NULL CHECK (octet_length(challenge) = 32),
  passkey_id          bigint        NOT NULL REFERENCES passkeys (id),
  assertion           jsonb         NOT NULL,
  created_at          timestamptz   NOT NULL DEFAULT now()
);
CREATE INDEX arbitration_rulings_case ON arbitration_rulings (case_id, id);
ALTER TABLE arbitration_cases
  ADD CONSTRAINT arbitration_cases_final_ruling_fk FOREIGN KEY (final_ruling_id) REFERENCES arbitration_rulings (id);

-- 当事人陈述与证据（只追加）：举证期内买卖双方提交文字与已处理完成的图片上传。
CREATE TABLE arbitration_statements (
  id         bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  case_id    bigint      NOT NULL REFERENCES arbitration_cases (id),
  user_id    bigint      NOT NULL REFERENCES users (id),
  body       text        NOT NULL CHECK (char_length(body) BETWEEN 1 AND 2000),
  upload_ids bigint[]    NOT NULL DEFAULT '{}' CHECK (cardinality(upload_ids) <= 9),
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX arbitration_statements_case ON arbitration_statements (case_id, id);

CREATE FUNCTION arbitration_append_only() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION '% is append-only', TG_TABLE_NAME;
END;
$$;
CREATE TRIGGER arbitration_rulings_no_update_delete
  BEFORE UPDATE OR DELETE ON arbitration_rulings
  FOR EACH ROW EXECUTE FUNCTION arbitration_append_only();
CREATE TRIGGER arbitration_rulings_no_truncate
  BEFORE TRUNCATE ON arbitration_rulings
  FOR EACH STATEMENT EXECUTE FUNCTION arbitration_append_only();
CREATE TRIGGER arbitration_statements_no_update_delete
  BEFORE UPDATE OR DELETE ON arbitration_statements
  FOR EACH ROW EXECUTE FUNCTION arbitration_append_only();
CREATE TRIGGER arbitration_statements_no_truncate
  BEFORE TRUNCATE ON arbitration_statements
  FOR EACH STATEMENT EXECUTE FUNCTION arbitration_append_only();
REVOKE UPDATE, DELETE, TRUNCATE ON arbitration_rulings, arbitration_statements FROM PUBLIC;

-- 能接案的人（ADR 0013 第 2 条：仲裁是管理员与站长的权限）。全系统只在这里判断角色：
-- staff_roles.role 1 = 站长、3 = 管理员；角色调整时只改这个函数。
CREATE FUNCTION arbitrator_eligible(p_user bigint) RETURNS boolean
LANGUAGE sql STABLE PARALLEL SAFE
RETURN EXISTS (SELECT 1 FROM staff_roles WHERE user_id = p_user AND role IN (1, 3));

-- 自动回避（plan 第 7 节）：本人是买家或卖家，或 90 天内（含仍在进行中）与任一方有过别的订单。
CREATE FUNCTION arbitrator_recused(p_case bigint, p_user bigint) RETURNS boolean
LANGUAGE sql STABLE PARALLEL SAFE
RETURN EXISTS (
  SELECT 1 FROM arbitration_cases c JOIN escrow_orders o ON o.id = c.order_id
  WHERE c.id = p_case AND (
    p_user = o.buyer_id OR p_user = o.seller_id
    OR EXISTS (
      SELECT 1 FROM escrow_orders x
      WHERE x.id <> o.id AND x.state <> 0
        AND (x.created_at > now() - interval '90 days' OR x.state IN (1, 2))
        AND ((x.buyer_id = p_user AND (x.seller_id = o.buyer_id OR x.seller_id = o.seller_id))
          OR (x.seller_id = p_user AND (x.buyer_id = o.buyer_id OR x.buyer_id = o.seller_id))))));
