-- 拍卖板块（站长反馈：找不到拍卖帖入口）：能否发拍卖帖从「允许交易帖」里拆出来，成为板块自己的开关。
-- 拍卖帖只能发在 allow_auction 的板块（挂拍建草稿与签名上线两步都查，forum-api）；交易帖仍看 allow_trade。
-- 默认关闭、不从 allow_trade 回填（安全优先：涉及钱的开关由站长在后台逐个打开，后台改它要 Passkey 再验证并写审计）。
-- 已在进行中的拍卖不受影响（出价、结算不看板块开关）；还没签名上线的草稿在没打开开关的板块里不能再上线。
--
-- 数据库账号（deploy/ansible 授权）：GRANT UPDATE (..., allow_auction) ON forums TO forum_api;
ALTER TABLE forums ADD COLUMN allow_auction boolean NOT NULL DEFAULT false;
