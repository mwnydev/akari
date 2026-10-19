import type { escrowAbi } from "@akari/contracts";
import type { ContractFunctionArgs, Hex } from "viem";

/** 链上 `Escrow.Offer`，类型直接取自生成的 ABI（不手抄字段）。 */
export type Offer = ContractFunctionArgs<typeof escrowAbi, "nonpayable", "createAndFund">[0];

/** 卖家签好的报价：私信里的报价卡片 / 付款链接携带的就是它。 */
export interface SignedOffer {
  offer: Offer;
  signature: Hex;
}

type FieldKind = "address" | "bigint" | "number" | "bool" | "bytes32";

const FIELDS: Record<keyof Offer, FieldKind> = {
  seller: "address",
  buyer: "address",
  token: "address",
  amount: "bigint",
  feePayer: "number",
  maxFeeBps: "number",
  maxBaseGas: "bigint",
  maxMinFee: "bigint",
  bond: "bigint",
  lateSlashBps: "number",
  instant: "bool",
  shipWindow: "number",
  confirmWindow: "number",
  listingHash: "bytes32",
  expiry: "bigint",
  maxFills: "number",
  nonce: "bigint",
};

const PATTERN: Record<FieldKind, RegExp> = {
  address: /^0x[0-9a-fA-F]{40}$/,
  bigint: /^\d{1,78}$/,
  number: /^\d{1,10}$/,
  bool: /^(true|false)$/,
  bytes32: /^0x[0-9a-fA-F]{64}$/,
};

const PARSE: Record<FieldKind, (v: string) => unknown> = {
  address: (v) => v,
  bigint: BigInt,
  number: Number,
  bool: (v) => v === "true",
  bytes32: (v) => v,
};

function toBase64Url(s: string): string {
  const bytes = new TextEncoder().encode(s);
  return btoa(String.fromCharCode(...bytes))
    .replaceAll("+", "-")
    .replaceAll("/", "_")
    .replace(/=+$/, "");
}

function fromBase64Url(s: string): string {
  const bin = atob(s.replaceAll("-", "+").replaceAll("_", "/"));
  return new TextDecoder().decode(Uint8Array.from(bin, (c) => c.charCodeAt(0)));
}

/** 编码成 URL 安全的字符串（付款链接 `?offer=`）。金额全部按十进制字符串存，不经过浮点。 */
export function encodeSignedOffer({ offer, signature }: SignedOffer): string {
  const o = Object.fromEntries(
    Object.keys(FIELDS).map((k) => [k, String(offer[k as keyof Offer])]),
  );
  return toBase64Url(JSON.stringify({ o, s: signature }));
}

/** 严格解码：任何字段缺失或格式不对都返回 null（链接可能被篡改，签名校验之前先挡掉畸形数据）。 */
export function decodeSignedOffer(code: string): SignedOffer | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(fromBase64Url(code));
  } catch {
    return null;
  }
  if (typeof parsed !== "object" || parsed === null) return null;
  const { o, s } = parsed as { o?: Record<string, unknown>; s?: unknown };
  if (!o || typeof s !== "string" || !/^0x([0-9a-fA-F]{2})+$/.test(s)) return null;

  const out: Record<string, unknown> = {};
  for (const [key, kind] of Object.entries(FIELDS)) {
    const v = o[key];
    if (typeof v !== "string" || !PATTERN[kind].test(v)) return null;
    out[key] = PARSE[kind](v);
  }
  return { offer: out as Offer, signature: s as Hex };
}

/** `FeeMath.Payer` 枚举值（Seller, Buyer, Half）与 @akari/fees 的 FeePayer 对应。 */
export const FEE_PAYERS = ["seller", "buyer", "half"] as const;
