// 拍卖的金额、加价与倒计时规则（纯函数，单元测试直接测）。金额一律是拍卖币种的最小单位：USDT 6 位小数 / 人民币分。
import type { Schemas } from "@akari/api-client";
import { cnyFenToUsdt } from "@akari/fees";
import { formatUsdt, parseUsdt } from "@akari/ui";
import { formatCnyFen, parseCnyFen } from "./fees";

export type AuctionView = Schemas["AuctionView"];
export type AuctionState = Schemas["AuctionState"];
export type AuctionBid = Schemas["BidView"];
export type AuctionParams = Schemas["AuctionParams"];
export type AuctionSettings = Schemas["AuctionSettings"];
export type AuctionEligibility = Schemas["Eligibility"];
export type NewAuctionRequest = Schemas["NewAuctionRequest"];

export type AuctionCurrency = "USDT" | "CNY";

/** 规则里的时长按整小时 / 整天显示。 */
export const hours = (secs: number) => Math.round(secs / 3600);
export const days = (secs: number) => Math.round(secs / 86_400);

/** 接口里的 `usdt` / `cny`；不认识的返回 null（按 USDT 显示可能让人看错金额，调用方不显示价格）。 */
export function auctionCurrency(api: string): AuctionCurrency | null {
  if (api === "usdt") return "USDT";
  if (api === "cny") return "CNY";
  return null;
}

/** 「¥1,280.00」或「128.00 USDT」。 */
export function formatAuctionAmount(currency: AuctionCurrency, amount: bigint): string {
  return currency === "CNY" ? formatCnyFen(amount) : `${formatUsdt(amount)} USDT`;
}

/** 填进输入框的写法：不带符号和千分位（「1300.00」）。 */
export function plainAuctionAmount(currency: AuctionCurrency, amount: bigint): string {
  return currency === "CNY"
    ? `${amount / 100n}.${(amount % 100n).toString().padStart(2, "0")}`
    : formatUsdt(amount).replaceAll(",", "");
}

export function parseAuctionAmount(currency: AuctionCurrency, input: string): bigint | null {
  return currency === "CNY" ? parseCnyFen(input) : parseUsdt(input);
}

/**
 * 成交时合约建单的 USDT 金额（最小单位）：USDT 拍卖就是成交价，人民币拍卖按卖家签名汇率向上取整
 * （@akari/fees 与合约 FeeMath.cnyFenToUsdt 共用 test-vectors/fees.json）。
 */
export function auctionUsdt(currency: AuctionCurrency, amount: bigint, cnyPerUsdt: number | null) {
  if (currency === "USDT") return amount;
  if (cnyPerUsdt === null) throw new Error("CNY auction without a rate");
  return cnyFenToUsdt(amount, BigInt(cnyPerUsdt));
}

const ONE_USDT = 1_000_000n;

/** 1 USDT 合多少拍卖币种单位（人民币向上取整到分，保证折合不低于 1 USDT）。 */
function oneUsdtIn(currency: AuctionCurrency, cnyPerUsdt: number | null): bigint {
  if (currency === "USDT") return ONE_USDT;
  if (cnyPerUsdt === null) throw new Error("CNY auction without a rate");
  // 1 USDT = rate / 10^4 元 = rate / 100 分
  return (BigInt(cnyPerUsdt) + 99n) / 100n;
}

const max = (...v: bigint[]) => v.reduce((a, b) => (b > a ? b : a));

/** 卖家能设的最低加价幅度：1 USDT 或起拍价的 1%（取大，plan 第 9 节「拍卖默认」）。 */
export function minIncrement(
  currency: AuctionCurrency,
  startPrice: bigint,
  cnyPerUsdt: number | null,
): bigint {
  return max(oneUsdtIn(currency, cnyPerUsdt), (startPrice + 99n) / 100n);
}

/**
 * 下一口最低出价：没人出过价时是起拍价，否则当前价加「卖家幅度、1 USDT、当前价 1%」三者中最大的
 * （与后端 `forum_core::auction::min_next_bid` 相同；推送来新出价时据此更新，出价以后端校验为准）。
 */
export function minNextBid(a: {
  currency: AuctionCurrency;
  cnyPerUsdt: number | null;
  start: bigint;
  increment: bigint;
  current: bigint | null;
}): bigint {
  if (a.current === null) return a.start;
  return a.current + max(a.increment, minIncrement(a.currency, a.current, a.cnyPerUsdt));
}

/**
 * 服务端时钟减本机时钟（毫秒）：`server_time` 取自请求往返的中点，抵消一半网络延迟。
 * 倒计时按服务端时钟算，用户本机时间不准也不会提前或推迟显示结束。
 */
export function clockOffsetMs(serverTimeIso: string, sentAtMs: number, receivedAtMs: number) {
  return Date.parse(serverTimeIso) - (sentAtMs + receivedAtMs) / 2;
}

/** 本机时刻 `nowMs` 时距服务端时刻 `iso` 还剩多少整秒（已过返回 0）。 */
export function secondsUntil(iso: string, nowMs: number, offsetMs: number): number {
  return Math.max(0, Math.floor((Date.parse(iso) - (nowMs + offsetMs)) / 1000));
}

/** 服务端渲染与水合首帧：直接用响应里的 server_time，不依赖本机时钟，两边结果一致。 */
export function secondsAtResponse(iso: string, serverTimeIso: string): number {
  return Math.max(0, Math.floor((Date.parse(iso) - Date.parse(serverTimeIso)) / 1000));
}
