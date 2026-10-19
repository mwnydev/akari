/** bps → 「62.5%」。 */
export function percent(bps: number): string {
  return `${(bps / 100).toFixed(2).replace(/\.?0+$/, "")}%`;
}

/** 按时率；还没有提交过时显示「—」。 */
export function onTimeRate(onTime: number, ruled: number): string {
  return ruled > 0 ? `${Math.round((onTime / ruled) * 100)}%` : "—";
}

/** Safe 批量文件名：链 ID + 本地日期，签名人按天对照。 */
export function batchFileName(chainId: number, at: Date): string {
  const d = [at.getFullYear(), at.getMonth() + 1, at.getDate()]
    .map((n) => String(n).padStart(2, "0"))
    .join("");
  return `akari-arbitration-${chainId}-${d}.json`;
}
