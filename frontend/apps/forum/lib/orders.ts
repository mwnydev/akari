// 订单状态与确认级别的中文（后端 `OrderView.state` / `level`）。

export const ORDER_STATE: Record<string, string> = {
  funded: "已付款",
  disputed: "争议中",
  released: "已放款",
  refunded: "已退款",
  settled: "已和解结算",
  none: "已撤销（下单交易被区块回滚）",
};

export const ORDER_LEVEL: Record<string, string> = {
  latest: "已上链",
  safe: "已安全确认",
  finalized: "已最终确认",
};
