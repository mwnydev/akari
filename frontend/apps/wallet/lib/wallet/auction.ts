import type { escrowAbi } from "@akari/contracts";
import { cnyFenToUsdt, isValidCnyPerUsdt } from "@akari/fees/cny";
import type { ContractFunctionArgs, Hex } from "viem";
import type { ApiAuctionAuth, ApiSettlementProof, AuctionSigning, AuctionView } from "./api";
import { chain } from "./config";

type CreateFromAuctionArgs = ContractFunctionArgs<
  typeof escrowAbi,
  "nonpayable",
  "createFromAuction"
>;

/** 链上 `Escrow.AuctionAuth` / `SettlementProof`，类型直接取自生成的 ABI（不手抄字段）。 */
export type AuctionAuth = CreateFromAuctionArgs[0];
export type SettlementProof = CreateFromAuctionArgs[2];

/** 与 Escrow.CURRENCY_USDT / CURRENCY_CNY 相同（ADR 0011）。 */
export const CURRENCY_USDT = 0;
export const CURRENCY_CNY = 1;

const ADDRESS = /^0x[0-9a-fA-F]{40}$/;
const BYTES32 = /^0x[0-9a-fA-F]{64}$/;
const UINT256 = /^\d{1,78}$/;

const isAddress = (v: unknown): v is Hex => typeof v === "string" && ADDRESS.test(v);
const isBytes32 = (v: unknown): v is Hex => typeof v === "string" && BYTES32.test(v);
const isUint256 = (v: unknown): v is string => typeof v === "string" && UINT256.test(v);
const isUint = (v: unknown, max: number): v is number =>
  typeof v === "number" && Number.isSafeInteger(v) && v >= 0 && v <= max;

const U8 = 0xff;
const U16 = 0xffff;
const U32 = 0xffff_ffff;

/**
 * 严格解析拍卖授权：任何字段缺失或格式不对、币种与汇率对不上（USDT 必须为 0，人民币须在合约范围内）都返回 null。
 * 确认页显示的、签名的、付款时提交的都是这里返回的同一个对象。
 */
export function parseAuctionAuth(json: ApiAuctionAuth): AuctionAuth | null {
  const a = json as unknown as Record<string, unknown>;
  if (
    !isAddress(a.seller) ||
    !isAddress(a.token) ||
    !isUint256(a.auctionId) ||
    !isUint256(a.reservePrice) ||
    !isUint(a.currency, U8) ||
    !isUint(a.cnyPerUsdt, U32) ||
    !isUint(a.feePayer, 2) ||
    !isUint(a.maxFeeBps, U16) ||
    !isUint256(a.maxBaseGas) ||
    !isUint256(a.maxMinFee) ||
    typeof a.instant !== "boolean" ||
    !isUint(a.shipWindow, U32) ||
    !isUint(a.confirmWindow, U32) ||
    !isBytes32(a.listingHash) ||
    !isUint(a.expiry, Number.MAX_SAFE_INTEGER)
  ) {
    return null;
  }
  const usdt = a.currency === CURRENCY_USDT && a.cnyPerUsdt === 0;
  const cny = a.currency === CURRENCY_CNY && isValidCnyPerUsdt(BigInt(a.cnyPerUsdt));
  if (!usdt && !cny) return null;
  return {
    seller: a.seller,
    token: a.token,
    auctionId: BigInt(a.auctionId),
    reservePrice: BigInt(a.reservePrice),
    currency: a.currency,
    cnyPerUsdt: a.cnyPerUsdt,
    feePayer: a.feePayer,
    maxFeeBps: a.maxFeeBps,
    maxBaseGas: BigInt(a.maxBaseGas),
    maxMinFee: BigInt(a.maxMinFee),
    instant: a.instant,
    shipWindow: a.shipWindow,
    confirmWindow: a.confirmWindow,
    listingHash: a.listingHash,
    expiry: BigInt(a.expiry),
  };
}

export function parseSettlementProof(json: ApiSettlementProof): SettlementProof | null {
  const p = json as unknown as Record<string, unknown>;
  if (
    !isBytes32(p.authHash) ||
    !isAddress(p.winner) ||
    !isUint256(p.price) ||
    !isUint(p.payDeadline, Number.MAX_SAFE_INTEGER)
  ) {
    return null;
  }
  return {
    authHash: p.authHash,
    winner: p.winner,
    price: BigInt(p.price),
    payDeadline: BigInt(p.payDeadline),
  };
}

/** 与 Escrow._auctionAmount 相同：USDT 拍卖就是成交价，人民币按卖家签名汇率向上取整（@akari/fees 与合约共用测试向量）。 */
export function auctionOrderAmount(auth: AuctionAuth, price: bigint): bigint {
  return auth.currency === CURRENCY_CNY ? cnyFenToUsdt(price, BigInt(auth.cnyPerUsdt)) : price;
}

/** Escrow.MIN_WINDOW / MAX_WINDOW：发货期、确认期的合约范围。 */
const MIN_WINDOW = 86_400;
const MAX_WINDOW = 30 * 86_400;

const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

/** 待签授权的签名域与它所属的拍卖（`AuctionSigning` 里与链有关的部分）。 */
export interface SigningTarget {
  auctionId: string;
  chainId: number;
  escrow: string | null | undefined;
}

/**
 * 签拍卖授权之前、不用读链就能做的核对（确认页渲染前就做，不对就不显示签名按钮）：
 * 网络与担保合约、卖家是本钱包、币种合约、拍卖编号、条款在合约范围内、还没过期。返回给用户看的原因，没问题返回 null。
 */
export function authorizationProblem(
  target: SigningTarget,
  auth: AuctionAuth,
  ctx: { chainId: number; escrow: string; token: string; wallet: string; nowSecs: number },
): string | null {
  if (target.chainId !== ctx.chainId || !target.escrow || !same(target.escrow, ctx.escrow)) {
    return "这份拍卖授权不属于当前网络的担保合约，请不要签名";
  }
  if (!same(auth.seller, ctx.wallet)) return "拍卖授权里的卖家不是你的钱包，请不要签名";
  if (!same(auth.token, ctx.token)) return "拍卖授权的结算币种不是 USDT，请不要签名";
  if (auth.auctionId.toString() !== target.auctionId) return "拍卖编号对不上，请回到论坛重新发起";
  const window = (w: number) => w >= MIN_WINDOW && w <= MAX_WINDOW;
  if (!window(auth.confirmWindow) || (!auth.instant && !window(auth.shipWindow))) {
    return "发货期或确认期超出合约允许的 1–30 天";
  }
  if (auth.expiry <= BigInt(ctx.nowSecs)) return "这份拍卖授权已过期，请回到论坛重新发起";
  return null;
}

/**
 * 授权里的条款必须与卖家在论坛填的拍卖一致（保留价、币种与汇率、手续费承担、发货与确认期）：
 * 两边都来自后端，对不上说明后端出错或被篡改，不签。
 */
export function termsProblem(auth: AuctionAuth, view: AuctionView): string | null {
  const currency = view.currency === "cny" ? CURRENCY_CNY : CURRENCY_USDT;
  const t = view.terms;
  if (
    view.reserve_price == null ||
    auth.reservePrice.toString() !== view.reserve_price ||
    auth.currency !== currency ||
    auth.cnyPerUsdt !== (view.cny_per_usdt ?? 0) ||
    auth.feePayer !== (t.fee_payer ?? 0) ||
    auth.instant !== (t.instant ?? false) ||
    auth.shipWindow !== t.ship_window ||
    auth.confirmWindow !== t.confirm_window
  ) {
    return "拍卖授权里的条款与你在论坛填写的不一致，请不要签名";
  }
  return null;
}

/** 拍卖授权的严格解析 + 不读链的核对（含与论坛拍卖条件的对照），签名确认页与签名前共用。 */
export function checkAuthorization(
  signing: AuctionSigning,
  view: AuctionView,
  walletAddress: string,
): AuctionAuth {
  const auth = parseAuctionAuth(signing.typed_data.message);
  if (!auth || signing.auction_id !== view.id || signing.typed_data.primaryType !== "AuctionAuth") {
    throw new Error("拍卖授权数据不完整，请回到论坛重新发起");
  }
  const { domain } = signing.typed_data;
  const target = {
    auctionId: String(view.id),
    chainId: domain.chainId,
    escrow: domain.verifyingContract,
  };
  const problem = authorizationProblem(target, auth, {
    chainId: chain.chainId,
    escrow: chain.contracts.Escrow,
    token: chain.contracts.MockUSDT,
    wallet: walletAddress,
    nowSecs: Math.floor(Date.now() / 1000),
  });
  const mismatch = problem ?? termsProblem(auth, view);
  if (mismatch) throw new Error(mismatch);
  return auth;
}
