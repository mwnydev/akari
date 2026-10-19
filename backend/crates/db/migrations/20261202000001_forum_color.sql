-- 板块色块（论坛侧栏、主题列表里板块名前的小方块）：固定色板里的一个 #rrggbb，与板块名一起显示、只作装饰。
-- 为空时前端按排序位置轮换。色板与 frontend/apps/forum/app/globals.css 的 --color-board-* 一致，改色板要两边一起改。
ALTER TABLE forums ADD COLUMN color text
  CONSTRAINT forums_color_palette
  CHECK (color IN ('#d1453b', '#2f7fd8', '#2e9d5c', '#e08e1b', '#8059c9', '#1d9bb0'));
