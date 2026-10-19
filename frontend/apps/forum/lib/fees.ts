import { DEFAULT_PARAMS, orderCost, type FeePayer } from "@akari/fees";

/** 交易帖上的费用预览（字符串便于直接交给 <Amount>）。 */
export function orderCostPreview(amount: string, feePayer: FeePayer) {
  const c = orderCost({ amount: BigInt(amount), feePayer, campaign: false, ...DEFAULT_PARAMS });
  return {
    sellerFee: (c.fee.seller + c.gas.seller).toString(),
    buyerPays: (c.buyerPays + DEFAULT_PARAMS.sponsorFee).toString(),
    disputeFee: c.disputeFee.toString(),
  };
}
