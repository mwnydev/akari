const USDT_DECIMALS = 6;
const SCALE = 10n ** BigInt(USDT_DECIMALS);

/**
 * 把 USDT 最小单位（6 位小数）格式化为 "1,234.50"：至少 2 位小数，去掉多余的尾随 0，
 * 不丢任何非零位（123.456789 原样显示）。只用 bigint，不经过浮点数。
 * 非法输入抛错，调用方不应传入未校验的字符串。
 */
export function formatUsdt(smallest: bigint | string): string {
  const v = typeof smallest === "bigint" ? smallest : parseSmallest(smallest);
  const neg = v < 0n;
  const abs = neg ? -v : v;
  const whole = (abs / SCALE).toString().replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  const frac = (abs % SCALE)
    .toString()
    .padStart(USDT_DECIMALS, "0")
    .replace(/0+$/, "")
    .padEnd(2, "0");
  return `${neg ? "-" : ""}${whole}.${frac}`;
}

function parseSmallest(s: string): bigint {
  if (!/^-?\d+$/.test(s)) throw new Error(`invalid USDT amount: ${s}`);
  return BigInt(s);
}

/**
 * 解析用户输入的 USDT 金额（如 "45"、"0.5"、"12.345678"）为最小单位；只用字符串与 bigint，不经过浮点。
 * 格式不对（负数、超过 6 位小数、千分位）返回 null。
 */
export function parseUsdt(input: string): bigint | null {
  const m = /^(\d{1,12})(?:\.(\d{0,6}))?$/.exec(input.trim());
  if (!m) return null;
  return BigInt(m[1] ?? "0") * SCALE + BigInt((m[2] ?? "").padEnd(USDT_DECIMALS, "0"));
}
