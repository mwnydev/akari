/** 解析用户输入的 USDT 金额为最小单位；只用字符串与 bigint，不经过浮点。格式不对返回 null。 */
export function parseUsdt(input: string): bigint | null {
  const m = /^(\d{1,12})(?:\.(\d{0,6}))?$/.exec(input.trim());
  if (!m) return null;
  return BigInt(m[1] ?? "0") * 1_000_000n + BigInt((m[2] ?? "").padEnd(6, "0"));
}
