-- 开发环境种子数据（不在迁移里，生产不执行）：
--   psql "$DATABASE_URL" -f crates/db/seeds/dev.sql
-- 可重复执行。生产的板块由后台创建（forum_db::forums::create）。
INSERT INTO forums (slug, name, description, position, view_min_level, post_min_level, reply_min_level) VALUES
  ('announcements', '站务公告', '规则变更、活动与维护通知', 0, 0, 99, 1),
  ('general',       '综合讨论', '什么都可以聊', 1, 0, 1, 1),
  ('tech',          '技术',     '编程、运维、区块链开发', 2, 0, 1, 1),
  ('trade',         '交易区',   '担保交易帖（发帖需开通交易）', 3, 0, 1, 1),
  ('auction',       '拍卖区',   '限时竞拍，成交后走担保交易（挂拍需开通交易）', 3, 0, 1, 1),
  ('help',          '帮助',     '使用问题与反馈', 4, 0, 1, 1),
  ('lounge',        '会员茶馆', '3 级以上可见', 5, 3, 3, 3)
ON CONFLICT (slug) DO NOTHING;
-- 交易帖只能发在交易区，拍卖帖只能发在拍卖区（与交易区同排序位置，排在它后面）。
UPDATE forums SET allow_trade = true WHERE slug = 'trade';
UPDATE forums SET allow_auction = true WHERE slug = 'auction';
-- 板块色块（固定色板，见迁移 20261202000001_forum_color）。
UPDATE forums SET color = v.color FROM (VALUES
  ('announcements', '#d1453b'), ('general', '#2f7fd8'), ('tech', '#8059c9'),
  ('trade', '#2e9d5c'), ('auction', '#e08e1b'), ('help', '#1d9bb0'), ('lounge', '#e08e1b')
) AS v (slug, color) WHERE forums.slug = v.slug AND forums.color IS NULL;
