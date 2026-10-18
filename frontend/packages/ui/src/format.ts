const USDT_DECIMALS = 6;
const SCALE = 10n ** BigInt(USDT_DECIMALS);

/**
 * 把 USDT 最小单位（6 位小数）格式化为 "1,234.500000"。只用 bigint，不经过浮点数。
 * 非法输入抛错，调用方不应传入未校验的字符串。
 */
export function formatUsdt(smallest: bigint | string): string {
  const v = typeof smallest === "bigint" ? smallest : parseSmallest(smallest);
  const neg = v < 0n;
  const abs = neg ? -v : v;
  const whole = (abs / SCALE).toString().replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  const frac = (abs % SCALE).toString().padStart(USDT_DECIMALS, "0");
  return `${neg ? "-" : ""}${whole}.${frac}`;
}

function parseSmallest(s: string): bigint {
  if (!/^-?\d+$/.test(s)) throw new Error(`invalid USDT amount: ${s}`);
  return BigInt(s);
}
