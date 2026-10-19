-- 人民币计价（ADR 0011）：拍卖成交的币种、卖家汇率与成交价；交易帖可选的卖家参考汇率。
-- 汇率为每 1 USDT 合多少元的 4 位小数定点数（7.2000 = 72000），范围与合约 MIN/MAX_CNY_PER_USDT 一致。

-- 拍卖成交订单（AuctionSettled 事件）：amount 仍是按卖家汇率换算后的 USDT 金额。
ALTER TABLE escrow_orders
  ADD COLUMN auction_currency     smallint CHECK (auction_currency IN (0, 1)),  -- 0 = USDT、1 = 人民币
  ADD COLUMN auction_cny_per_usdt int,                                           -- USDT 拍卖为 0
  ADD COLUMN auction_price        numeric(78,0);                                 -- 拍卖币种：USDT 最小单位或分

-- 交易帖的人民币参考价只供显示，不进内容哈希；报价与订单仍以 USDT 为准。
ALTER TABLE trade_listings
  ADD COLUMN cny_per_usdt int CHECK (cny_per_usdt BETWEEN 50000 AND 100000);
