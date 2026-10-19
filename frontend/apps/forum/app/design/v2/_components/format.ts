/** 浏览量等计数：一万以上显示「1.3万」。 */
export function formatCount(n: number): string {
  if (n < 10_000) return String(n);
  return `${(Math.round(n / 1000) / 10).toString()}万`;
}
