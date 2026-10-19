-- 我的订单（卖家），游标翻页、新的在前；不含已被区块回滚撤销的订单。$1 seller_id, $2 游标（上一页最后一个 id）, $3 条数
SELECT id FROM escrow_orders
WHERE seller_id = $1 AND id < $2 AND state <> 0
ORDER BY id DESC
LIMIT $3
