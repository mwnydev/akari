-- 管理员群告警去重（ADR 0013 第 6 条）：持续存在的状况（Paymaster 押金 / 热钱包余额不足、indexer 切到备用 RPC、
-- 越过 safe 的区块回滚）一段时间内只推送一次。按 `dedupe_key`（= 告警的 event）查，不看 payload：发送后 payload 会被清空。
--
-- 余额告警原先以 `alert.*` 类型写入发件箱，worker 不认识这种类型、从未送达；现在统一走 `staff_alert`。
--
-- 数据库账号（deploy/ansible）另需：
--   GRANT SELECT (id, dedupe_key, created_at) ON outbox TO forum_indexer, forum_sponsor;   -- RETURNING id 与去重查询；读不到 payload
ALTER TABLE outbox ADD COLUMN dedupe_key text;
CREATE INDEX outbox_dedupe ON outbox (dedupe_key, created_at) WHERE dedupe_key IS NOT NULL;
DROP INDEX outbox_alerts;
