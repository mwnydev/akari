// 人民币计价与卖家汇率换算，与合约 FeeMath、Rust 共用 test-vectors/fees.json 的 cny 部分。金额为 bigint。

/** 人民币汇率（ADR 0011）：每 1 USDT 合多少元，4 位小数定点数（7.2000 = 72000），范围与合约一致。 */
export const MIN_CNY_PER_USDT = 50_000n;
export const MAX_CNY_PER_USDT = 100_000n;
/** 10^6（USDT 最小单位）× 10^4（汇率小数位）÷ 100（每元的分数）。 */
const CNY_SCALE = 100_000_000n;

export function isValidCnyPerUsdt(rate: bigint): boolean {
  return rate >= MIN_CNY_PER_USDT && rate <= MAX_CNY_PER_USDT;
}

function checkRate(rate: bigint): void {
  if (!isValidCnyPerUsdt(rate)) throw new RangeError(`cnyPerUsdt out of range: ${rate}`);
}

/** 人民币成交价（分）按卖家汇率折合 USDT，向上取整（卖家所得不低于汇率换算值，与合约一致）。 */
export function cnyFenToUsdt(priceFen: bigint, cnyPerUsdt: bigint): bigint {
  checkRate(cnyPerUsdt);
  return (priceFen * CNY_SCALE + cnyPerUsdt - 1n) / cnyPerUsdt;
}

/** USDT 标价的人民币参考价（分），向下取整，只供显示。 */
export function usdtToCnyFen(amount: bigint, cnyPerUsdt: bigint): bigint {
  checkRate(cnyPerUsdt);
  return (amount * cnyPerUsdt) / CNY_SCALE;
}
