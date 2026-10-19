import type { escrowAbi } from "@akari/contracts";
import type { ContractFunctionArgs, Hex } from "viem";
import type { ApiOffer } from "./api";

/** 链上 `Escrow.Offer`，类型直接取自生成的 ABI（不手抄字段）。 */
export type Offer = ContractFunctionArgs<typeof escrowAbi, "nonpayable", "createAndFund">[0];

/** 卖家签好的报价：私信 / 交易帖里的报价卡片携带的就是它。 */
export interface SignedOffer {
  offer: Offer;
  signature: Hex;
}

type FieldKind = "address" | "uint256" | "uint" | "bool" | "bytes32";

/** 接口 JSON（forum_core::eip712::Offer）：uint256 为十进制字符串，较小的整数为 JSON 数字。 */
const FIELDS: Record<keyof Offer, FieldKind> = {
  seller: "address",
  buyer: "address",
  token: "address",
  amount: "uint256",
  feePayer: "uint",
  maxFeeBps: "uint",
  maxBaseGas: "uint256",
  maxMinFee: "uint256",
  bond: "uint256",
  lateSlashBps: "uint",
  instant: "bool",
  shipWindow: "uint",
  confirmWindow: "uint",
  listingHash: "bytes32",
  expiry: "uint",
  maxFills: "uint",
  nonce: "uint256",
};

const ADDRESS = /^0x[0-9a-fA-F]{40}$/;
const BYTES32 = /^0x[0-9a-fA-F]{64}$/;
const UINT256 = /^\d{1,78}$/;

/** 严格解析：任何字段缺失或格式不对都返回 null（签名校验之前先挡掉畸形数据）。 */
export function parseOffer(json: ApiOffer): Offer | null {
  const src = json as unknown as Record<string, unknown>;
  const out: Record<string, unknown> = {};
  for (const [key, kind] of Object.entries(FIELDS)) {
    const v = src[key];
    switch (kind) {
      case "address":
        if (typeof v !== "string" || !ADDRESS.test(v)) return null;
        out[key] = v;
        break;
      case "bytes32":
        if (typeof v !== "string" || !BYTES32.test(v)) return null;
        out[key] = v;
        break;
      case "uint256":
        if (typeof v !== "string" || !UINT256.test(v)) return null;
        out[key] = BigInt(v);
        break;
      case "uint":
        if (typeof v !== "number" || !Number.isSafeInteger(v) || v < 0) return null;
        // expiry 在 ABI 里是 uint64（bigint），其余是 uint8–uint32（number）
        out[key] = key === "expiry" ? BigInt(v) : v;
        break;
      case "bool":
        if (typeof v !== "boolean") return null;
        out[key] = v;
        break;
    }
  }
  return out as Offer;
}

/** 提交给 `POST /api/v1/offers` 的 JSON：金额全部按十进制字符串，不经过浮点。 */
export function offerToJson(offer: Offer): ApiOffer {
  return {
    ...offer,
    amount: offer.amount.toString(),
    maxBaseGas: offer.maxBaseGas.toString(),
    maxMinFee: offer.maxMinFee.toString(),
    bond: offer.bond.toString(),
    nonce: offer.nonce.toString(),
    expiry: Number(offer.expiry),
  };
}

/** `FeeMath.Payer` 枚举值（Seller, Buyer, Half）与 @akari/fees 的 FeePayer 对应。 */
export const FEE_PAYERS = ["seller", "buyer", "half"] as const;
