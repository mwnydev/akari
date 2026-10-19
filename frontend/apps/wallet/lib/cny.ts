// 拍卖人民币计价的显示（ADR 0011）。单独成模块：只进拍卖页面的分包，不进其他页面的首屏 JS。

/** 人民币金额（分）→ "¥1,234.56"。 */
export function formatCnyFen(fen: bigint): string {
  const yuan = (fen / 100n).toString().replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  return `¥${yuan}.${(fen % 100n).toString().padStart(2, "0")}`;
}

/** 汇率的 4 位小数定点数 → "7.2000"。 */
export function formatCnyPerUsdt(rate: number): string {
  return `${Math.floor(rate / 10_000)}.${String(rate % 10_000).padStart(4, "0")}`;
}
