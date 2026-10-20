-- 发件箱逐条隔离（第 9 轮评审：一条违反约束的通知让整批回滚，所有通知和邮件都停了）。
--
-- worker 先认领（`attempts + 1`，`next_attempt_at` 推后一个租期），再逐条投递、逐条标记；认领后崩溃的消息租期一过
-- 重新到期。`attempts` 同时是这次认领的令牌：标记时核对，租期过后被别的 worker 重新认领的，原认领者的标记不生效。
-- 重试不会成功的错误（违反约束、payload 不对、不认识的类型）与重试用尽的消息进死信（`failed_at`），保留 payload，
-- 站长在后台重新投递或丢弃（`discarded_at`，清空 payload）；死信 14 天后自动丢弃。
--
-- 同一订单的 `order.updated` 按 ID 顺序投递（推送里带订单状态，乱序会让页面停在旧状态）：`ordering_key` 相同的消息
-- 只有最早一条未完成的可以认领，只挡住这一个订单，不挡别的消息。投递后 payload 清空，`ordering_key` 随之变成 NULL。
--
-- 数据库账号（deploy/ansible）另需：
--   GRANT UPDATE (discarded_at) ON outbox TO forum_worker;                      -- 死信过期丢弃
--   GRANT SELECT (kind, attempts, last_error, sent_at, failed_at, discarded_at) ON outbox TO forum_api;  -- 后台死信列表
--   GRANT UPDATE (attempts, next_attempt_at, failed_at, discarded_at, payload) ON outbox TO forum_api;   -- 重新投递 / 丢弃
ALTER TABLE outbox
  ADD COLUMN discarded_at timestamptz,
  ADD COLUMN ordering_key text GENERATED ALWAYS AS (
    CASE WHEN kind = 'order.updated' THEN 'order:' || (payload ->> 'id') END
  ) STORED,
  ADD CONSTRAINT outbox_discarded_is_dead CHECK (discarded_at IS NULL OR failed_at IS NOT NULL);

-- 之前放弃的消息投递失败时已清空 payload，重新投递也没有内容：直接算作丢弃。
UPDATE outbox SET discarded_at = failed_at WHERE failed_at IS NOT NULL;

CREATE INDEX outbox_ordering ON outbox (ordering_key, id)
  WHERE sent_at IS NULL AND failed_at IS NULL AND ordering_key IS NOT NULL;
CREATE INDEX outbox_dead ON outbox (id)
  WHERE failed_at IS NOT NULL AND discarded_at IS NULL;
