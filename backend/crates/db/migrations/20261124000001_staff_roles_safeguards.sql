-- 管理角色与兜底措施（ADR 0013）：角色只保留站长 1、管理员 3、版主 4；处罚、管理权限暂停、站点设置、可撤销的审计日志。

-- 超级管理员并入站长、仲裁员并入管理员。同一用户已有目标角色时先删掉重复行，再改编号。
DELETE FROM staff_roles s
WHERE s.role IN (2, 5)
  AND EXISTS (
    SELECT 1 FROM staff_roles t
    WHERE t.user_id = s.user_id AND t.role = CASE s.role WHEN 2 THEN 1 ELSE 3 END
  );
UPDATE staff_roles SET role = CASE role WHEN 2 THEN 1 ELSE 3 END WHERE role IN (2, 5);
ALTER TABLE staff_roles DROP CONSTRAINT staff_roles_role_check;
-- role：1=站长、3=管理员、4=版主（版主的 forum_id 为管辖板块）。编号与旧表一致，仲裁资格按 role IN (1, 3) 判断。
ALTER TABLE staff_roles ADD CONSTRAINT staff_roles_role_check CHECK (role IN (1, 3, 4));

-- 处罚：kind 1=禁言、2=禁拍、3=封号、4=IP 封禁、5=设备封禁。
-- 禁言、禁拍可限定板块（forum_id，为空表示全站）；ends_at 为空表示永久。撤销只记 revoked_*，不删行。
CREATE TABLE sanctions (
  id            bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  kind          smallint    NOT NULL CHECK (kind BETWEEN 1 AND 5),
  user_id       bigint      REFERENCES users (id),     -- 1–3 的对象；5 为设备所属用户（只用于展示）
  forum_id      int         REFERENCES forums (id),
  ip            cidr,
  device_hash   bytea       CHECK (octet_length(device_hash) = 32),
  ends_at       timestamptz,
  reason        text        NOT NULL,
  created_by    bigint      NOT NULL REFERENCES users (id),
  created_at    timestamptz NOT NULL DEFAULT now(),
  revoked_at    timestamptz,
  revoked_by    bigint      REFERENCES users (id),
  CHECK (ends_at IS NULL OR ends_at > created_at),
  CHECK (forum_id IS NULL OR kind IN (1, 2)),
  CHECK ((kind IN (1, 2, 3)) <= (user_id IS NOT NULL)),
  CHECK ((kind = 4) = (ip IS NOT NULL)),
  CHECK ((kind = 5) = (device_hash IS NOT NULL)),
  CHECK ((revoked_at IS NULL) = (revoked_by IS NULL))
);
CREATE INDEX sanctions_user_active ON sanctions (user_id, kind) WHERE revoked_at IS NULL;
CREATE INDEX sanctions_user ON sanctions (user_id, id DESC);
CREATE INDEX sanctions_ip_active ON sanctions USING gist (ip inet_ops) WHERE kind = 4 AND revoked_at IS NULL;
CREATE INDEX sanctions_device_active ON sanctions (device_hash) WHERE kind = 5 AND revoked_at IS NULL;

-- 管理权限暂停：短时间内删帖 / 处罚超过阈值时自动暂停，站长在后台确认后恢复。
CREATE TABLE staff_suspensions (
  id             bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  user_id        bigint      NOT NULL REFERENCES users (id),
  reason         text        NOT NULL,
  detail         jsonb       NOT NULL DEFAULT '{}',
  created_at     timestamptz NOT NULL DEFAULT now(),
  reinstated_at  timestamptz,
  reinstated_by  bigint      REFERENCES users (id),
  CHECK ((reinstated_at IS NULL) = (reinstated_by IS NULL))
);
CREATE UNIQUE INDEX staff_suspensions_active ON staff_suspensions (user_id) WHERE reinstated_at IS NULL;
-- 异常计数只算最近一次恢复之后的操作。
CREATE INDEX staff_suspensions_user ON staff_suspensions (user_id, reinstated_at);

-- 站长在后台修改的链下参数（plan 第 9 节）；值的范围由程序校验。
CREATE TABLE site_settings (
  key        text        PRIMARY KEY,
  value      jsonb       NOT NULL,
  updated_by bigint      REFERENCES users (id),
  updated_at timestamptz NOT NULL DEFAULT now()
);

-- 撤销记录指向被撤销的那条日志；每条最多撤销一次。
ALTER TABLE admin_audit_logs ADD COLUMN reverts bigint REFERENCES admin_audit_logs (id);
CREATE UNIQUE INDEX admin_audit_logs_reverts ON admin_audit_logs (reverts) WHERE reverts IS NOT NULL;
-- 异常检测（某账号窗口内的删帖 / 处罚次数）与一键撤销（某账号某时间段）都按操作者 + 时间查。
CREATE INDEX admin_audit_logs_actor ON admin_audit_logs (actor_id, created_at);
