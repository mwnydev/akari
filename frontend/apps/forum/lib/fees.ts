import type { Schemas } from "@akari/api-client";
import { DEFAULT_PARAMS, orderCost, type FeePayer } from "@akari/fees";

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
