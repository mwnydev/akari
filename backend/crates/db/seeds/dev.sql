-- 开发环境种子数据（不在迁移里，生产不执行）：
--   psql "$DATABASE_URL" -f crates/db/seeds/dev.sql
-- 可重复执行。生产的板块由后台创建（forum_db::forums::create）。
INSERT INTO forums (slug, name, description, position, view_min_level, post_min_level, reply_min_level) VALUES
  ('announcements', '站务公告', '规则变更、活动与维护通知', 0, 0, 99, 1),
  ('general',       '综合讨论', '什么都可以聊', 1, 0, 1, 1),
  ('tech',          '技术',     '编程、运维、区块链开发', 2, 0, 1, 1),
  ('trade',         '交易区',   '担保交易帖（发帖需开通交易）', 3, 0, 1, 1),
  ('help',          '帮助',     '使用问题与反馈', 4, 0, 1, 1),
  ('lounge',        '会员茶馆', '3 级以上可见', 5, 3, 3, 3)
ON CONFLICT (slug) DO NOTHING;
-- 交易帖只能发在交易区。
UPDATE forums SET allow_trade = true WHERE slug = 'trade';
