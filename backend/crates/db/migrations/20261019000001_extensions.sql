-- 迁移只增不改（plan 第 8 节）：已发布的迁移文件永不修改，变更一律新增文件。
--
-- 数据库账号分离（plan 第 2、8 节）属于部署配置，由 deploy/ansible 创建：
--   forum_api      论坛表读写；订单 / 链上事件等派生表只读
--   forum_worker   发件箱、任务表读写
--   forum_indexer  唯一能写 escrow_orders、escrow_events、user_trade_stats 的账号
--   forum_sponsor  只读所需表
-- 所有账号都不授予 admin_audit_logs 的 UPDATE / DELETE（另有触发器兜底，见审计日志迁移）。
CREATE EXTENSION IF NOT EXISTS pgroonga;
