-- 事务发件箱（plan 第 2 节）：业务数据与待发送通知同一事务写入，worker 再投递。
-- 投递成功后清空 payload（验证码等敏感内容不长期留在库里）。
CREATE TABLE outbox (
  id              bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  kind            text        NOT NULL,
  payload         jsonb       NOT NULL,
  priority        smallint    NOT NULL DEFAULT 0,   -- 越大越先发；验证码 = 10
  attempts        int         NOT NULL DEFAULT 0,
  next_attempt_at timestamptz NOT NULL DEFAULT now(),
  sent_at         timestamptz,
  failed_at       timestamptz,                      -- 超过重试上限后放弃
  last_error      text,
  created_at      timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX outbox_pending ON outbox (priority DESC, id)
  WHERE sent_at IS NULL AND failed_at IS NULL;

-- 只增审计日志（plan 第 9 节）：任何账号都改不了、删不了。
CREATE TABLE admin_audit_logs (
  id         bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  actor_id   bigint      NOT NULL,
  action     text        NOT NULL,
  target     text,
  reason     text,
  detail     jsonb       NOT NULL DEFAULT '{}',
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX admin_audit_logs_created ON admin_audit_logs USING brin (created_at);

CREATE FUNCTION admin_audit_logs_append_only() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'admin_audit_logs is append-only';
END;
$$;
CREATE TRIGGER admin_audit_logs_no_update_delete
  BEFORE UPDATE OR DELETE ON admin_audit_logs
  FOR EACH ROW EXECUTE FUNCTION admin_audit_logs_append_only();
CREATE TRIGGER admin_audit_logs_no_truncate
  BEFORE TRUNCATE ON admin_audit_logs
  FOR EACH STATEMENT EXECUTE FUNCTION admin_audit_logs_append_only();
REVOKE UPDATE, DELETE, TRUNCATE ON admin_audit_logs FROM PUBLIC;
