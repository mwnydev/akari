-- 出价（只在一条语句里完成，行锁只持有到语句结束）：锁住拍卖行；进行中、未到结束时间、出价人不是当前最高者、
-- 出价不低于「起拍价（第一口）/ 当前价 + max(加价幅度, 1 USDT, 当前价 1%)」才更新当前价与出价数，结束前窗口内的出价
-- 把结束时间顺延（不超过顺延上限）；同时写一行出价记录，并标出与本场其他出价人是否关联（同设备）。
-- 不满足任何一条时不返回行。$1 auction_id, $2 bidder_id, $3 amount, $4 amount_usdt, $5 拍卖币种里的 1 USDT
WITH a AS (
  SELECT id, top_bidder_id AS prev_bidder FROM auctions WHERE id = $1 FOR UPDATE
),
up AS (
  UPDATE auctions u SET
    top_amount = $3::text::numeric,
    top_bidder_id = $2,
    bid_count = u.bid_count + 1,
    ends_at = CASE WHEN u.ends_at - now() <= make_interval(secs => u.snipe_window_secs)
                   THEN GREATEST(u.ends_at, LEAST(u.ends_at + make_interval(secs => u.snipe_extension_secs), u.max_ends_at))
                   ELSE u.ends_at END,
    updated_at = now()
  FROM a
  WHERE u.id = a.id AND u.state = 1 AND u.ends_at > now() AND u.top_bidder_id IS DISTINCT FROM $2
    AND $3::text::numeric >= CASE
          WHEN u.top_amount IS NULL THEN u.start_price
          ELSE u.top_amount + GREATEST(u.increment, $5::text::numeric, ceil(u.top_amount / 100)) END
  RETURNING u.id, u.ends_at, u.bid_count, u.top_amount >= u.reserve_price AS reserve_met, a.prev_bidder
),
bid AS (
  INSERT INTO auction_bids (auction_id, bidder_id, amount, amount_usdt, linked)
  SELECT up.id, $2, $3::text::numeric, $4::text::numeric, EXISTS (
    -- 从出价人自己的设备找同设备的其他账号，再看他们有没有在本场出过价（热门拍卖的出价很多，不逐口比）。
    SELECT 1 FROM user_devices mine
    JOIN user_devices other ON other.device_hash = mine.device_hash AND other.user_id <> $2
    WHERE mine.user_id = $2
      AND EXISTS (SELECT 1 FROM auction_bids o WHERE o.auction_id = $1 AND o.bidder_id = other.user_id))
  FROM up
  RETURNING id, linked, created_at
)
SELECT bid.id AS "bid_id!", bid.linked AS "linked!", bid.created_at AS "created_at!", up.ends_at AS "ends_at!",
       up.bid_count AS "bid_count!", up.reserve_met AS "reserve_met!", up.prev_bidder
FROM up, bid
