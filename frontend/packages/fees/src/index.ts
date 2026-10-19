// 费用与分期解锁规则，与合约、Rust 共用 test-vectors/fees.json。金额一律为 USDT 最小单位（6 位小数）的 bigint。

export type FeePayer = "seller" | "buyer" | "half";

const BPS = 10_000n;

export interface Split {
  buyer: bigint;
  seller: bigint;
}

export function split(total: bigint, payer: FeePayer): Split {
  switch (payer) {
    case "seller":
      return { buyer: 0n, seller: total };
    case "buyer":
      return { buyer: total, seller: 0n };
    case "half": {
      const buyer = total / 2n;
      return { buyer, seller: total - buyer };
    }
  }
}

/** 手续费总额：活动期间按活动费率且最低手续费失效，否则 max(金额 × 费率, 最低手续费)。 */
export function feeTotal(
  amount: bigint,
  feeBps: bigint,
  minFee: bigint,
  campaign: boolean,
): bigint {
  const byRate = (amount * feeBps) / BPS;
  if (campaign) return byRate;
  return byRate > minFee ? byRate : minFee;
}

/** 争议费 = (手续费 > 0 ? 手续费 : 最低手续费) × 倍数。 */
export function disputeFee(feeTotalAmount: bigint, minFee: bigint, disputeMulBps: bigint): bigint {
  const base = feeTotalAmount > 0n ? feeTotalAmount : minFee;
  return (base * disputeMulBps) / BPS;
}

export interface OrderCost {
  fee: Split;
  gas: Split;
  buyerPays: bigint;
  disputeFee: bigint;
}

export interface OrderParams {
  amount: bigint;
  feePayer: FeePayer;
  feeBps: bigint;
  minFee: bigint;
  campaign: boolean;
  baseGas: bigint;
  disputeMulBps: bigint;
}

export function orderCost(p: OrderParams): OrderCost {
  const total = feeTotal(p.amount, p.feeBps, p.minFee, p.campaign);
  const fee = split(total, p.feePayer);
  const gas = split(p.baseGas, p.feePayer);
  return {
    fee,
    gas,
    buyerPays: p.amount + fee.buyer + gas.buyer,
    disputeFee: disputeFee(total, p.minFee, p.disputeMulBps),
  };
}

/** 分期可解锁净本金 = 金额 − 卖家承担的手续费 − 卖家承担的基础 gas 费。 */
export function streamNet(sellerFee: bigint, sellerGas: bigint, amount: bigint): bigint {
  return amount - sellerFee - sellerGas;
}

/** 已解锁 = 净本金 × min(已过周期数, 总期数) ÷ 总期数（向下取整，余数归最后一期）。 */
export function streamUnlocked(net: bigint, elapsedPeriods: bigint, periods: bigint): bigint {
  const e = elapsedPeriods < periods ? elapsedPeriods : periods;
  return (net * e) / periods;
}

/** 本次 claim 卖家实收；可领取须 > 领取 gas 费，否则返回 null（不可领取）。 */
export function claimPayout(unlocked: bigint, claimed: bigint, claimGas: bigint): bigint | null {
  const claimable = unlocked - claimed;
  return claimable > claimGas ? claimable - claimGas : null;
}

/** plan 第 9 节链上参数默认值。上线后以链上 / API 返回的参数为准，这里只供设计稿和测试使用。 */
export const DEFAULT_PARAMS = {
  feeBps: 100n,
  minFee: 1_000_000n,
  baseGas: 100_000n,
  claimGas: 20_000n,
  disputeMulBps: 10_000n,
  /** 付款前操作的代付费，由 Paymaster 在验证阶段预扣（ADR 0004），不进担保合约 */
  sponsorFee: 50_000n,
} as const;

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
