import type { Schemas } from "@akari/api-client";
import {
  DEFAULT_PARAMS,
  isValidCnyPerUsdt,
  orderCost,
  usdtToCnyFen,
  type FeePayer,
} from "@akari/fees";

/** ListingTerms.fee_payer / Offer.feePayer 的编码：0 卖家、1 买家、2 各半。 */
export const FEE_PAYERS: readonly { value: FeePayer; label: string }[] = [
  { value: "seller", label: "卖家承担" },
  { value: "buyer", label: "买家承担" },
  { value: "half", label: "双方各半" },
];

export function feePayerOf(code: number | undefined): FeePayer {
  return FEE_PAYERS[code ?? 0]?.value ?? "seller";
}

/**
 * 交易帖上的费用预览（字符串便于直接交给 <Amount>）。费率、最低手续费、固定 gas 取自 /trade/params（链上当前值）；
 * 代付费与争议费倍数接口里没有，用 plan 第 9 节的默认值。
 */
export function orderCostPreview(
  amount: string,
  feePayer: FeePayer,
  params: Pick<Schemas["TradeParams"], "fee_bps" | "min_fee" | "base_gas">,
) {
  const c = orderCost({
    ...DEFAULT_PARAMS,
    amount: BigInt(amount),
    feePayer,
    campaign: false,
    feeBps: BigInt(params.fee_bps),
    minFee: BigInt(params.min_fee),
    baseGas: BigInt(params.base_gas),
  });
  return {
    sellerFee: (c.fee.seller + c.gas.seller).toString(),
    buyerPays: (c.buyerPays + DEFAULT_PARAMS.sponsorFee).toString(),
    disputeFee: c.disputeFee.toString(),
  };
}

/** 汇率的 4 位小数定点数（ADR 0011）：72000 ↔ "7.2000"。 */
const RATE_DECIMALS = 4;

/** 解析卖家填写的参考汇率（如 "7.2"、"7.1234"）；格式不对或超出 5.0000–10.0000 返回 null。 */
export function parseCnyPerUsdt(input: string): bigint | null {
  const m = /^(\d{1,2})(?:\.(\d{0,4}))?$/.exec(input.trim());
  if (!m) return null;
  const rate = BigInt(`${m[1] ?? "0"}${(m[2] ?? "").padEnd(RATE_DECIMALS, "0")}`);
  return isValidCnyPerUsdt(rate) ? rate : null;
}

export function formatCnyPerUsdt(rate: number): string {
  return (rate / 10 ** RATE_DECIMALS).toFixed(RATE_DECIMALS);
}

/** 解析用户输入的人民币金额（如 "1300"、"1300.5"）为分；格式不对返回 null。 */
export function parseCnyFen(input: string): bigint | null {
  const m = /^(\d{1,10})(?:\.(\d{0,2}))?$/.exec(input.trim());
  if (!m) return null;
  return BigInt(m[1] ?? "0") * 100n + BigInt((m[2] ?? "").padEnd(2, "0"));
}

/** 人民币金额（分）→ "¥1,234.56"。 */
export function formatCnyFen(fen: bigint): string {
  const yuan = (fen / 100n).toString().replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  return `¥${yuan}.${(fen % 100n).toString().padStart(2, "0")}`;
}

/** USDT 标价按卖家参考汇率折合的人民币参考价，如 "¥1,234.56"。 */
export function cnyReference(usdt: string, rate: number): string {
  return formatCnyFen(usdtToCnyFen(BigInt(usdt), BigInt(rate)));
}
