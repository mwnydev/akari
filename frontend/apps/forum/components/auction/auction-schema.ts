import type { Schemas } from "@akari/api-client";
import { formatUsdt, parseUsdt } from "@akari/ui";
import * as z from "zod/mini";
import {
  auctionUsdt,
  formatAuctionAmount,
  minIncrement,
  parseAuctionAmount,
  type AuctionCurrency,
  type AuctionEligibility,
  type AuctionSettings,
  type NewAuctionRequest,
} from "../../lib/auction";
import { parseCnyPerUsdt } from "../../lib/fees";

const HOUR = 3600;
const DAY = 86_400;

/** 拍卖时长的可选档位；只显示站长设定范围内的（plan 第 9 节，默认 24 小时–7 天）。 */
const DURATIONS = [
  HOUR,
  6 * HOUR,
  12 * HOUR,
  DAY,
  2 * DAY,
  3 * DAY,
  5 * DAY,
  7 * DAY,
  14 * DAY,
  30 * DAY,
];

/** 站长设定的拍卖时长范围（`GET /auctions/params` 的 `settings`）。 */
type DurationBounds = {
  settings: Pick<AuctionSettings, "min_duration_secs" | "max_duration_secs">;
};

export function durationOptions({ settings: s }: DurationBounds) {
  return DURATIONS.filter((d) => d >= s.min_duration_secs && d <= s.max_duration_secs).map((d) => ({
    value: String(d),
    label: d < DAY ? `${d / HOUR} 小时` : `${d / DAY} 天`,
  }));
}

/** 出价资格预设（plan 第 9 节「拍卖默认」）。 */
export const ELIGIBILITY_OPTIONS = [
  { value: "none", label: "不要求" },
  { value: "10", label: "钱包余额 ≥ 出价的 10%" },
  { value: "50", label: "钱包余额 ≥ 出价的 50%" },
  { value: "100", label: "钱包余额 ≥ 出价的 100%" },
  { value: "fixed", label: "钱包余额 ≥ 固定金额" },
] as const;

export type AuctionLimits = Pick<
  Schemas["TradeParams"],
  "min_order" | "order_cap" | "min_window" | "max_window"
> & { auction: DurationBounds };

interface Prices {
  currency: AuctionCurrency;
  rate: number | null;
  start: bigint | null;
  increment: bigint | null;
  reserve: bigint | null;
}

/** 表单里的价格字段解析成拍卖币种最小单位；人民币汇率不对时 rate 为 null（金额校验先等汇率）。 */
export function auctionPrices(v: {
  currency: AuctionCurrency;
  cnyRate: string;
  start: string;
  increment: string;
  reserve: string;
}): Prices {
  const rate = v.currency === "CNY" ? parseCnyPerUsdt(v.cnyRate) : null;
  const amount = (s: string) => {
    const a = parseAuctionAmount(v.currency, s);
    return a !== null && a > 0n ? a : null;
  };
  return {
    currency: v.currency,
    rate: rate === null ? null : Number(rate),
    start: amount(v.start),
    increment: amount(v.increment),
    // 不填保留价 = 保留价等于起拍价（有人出价就成交）
    reserve: v.reserve.trim() === "" ? amount(v.start) : amount(v.reserve),
  };
}

/** 价格折合 USDT（合约建单金额）；人民币汇率还没填对时返回 null。 */
export function pricesUsdt(p: Prices, amount: bigint | null): bigint | null {
  if (amount === null) return null;
  if (p.currency === "CNY" && p.rate === null) return null;
  return auctionUsdt(p.currency, amount, p.rate);
}

/** 发拍卖帖表单的校验（金额上下限按折合 USDT 与链上单笔限额比较，加价幅度不低于 max(1 USDT, 起拍价 1%)）。 */
export function auctionSchema(limits: AuctionLimits) {
  const minDays = Math.ceil(limits.min_window / DAY);
  const maxDays = Math.floor(limits.max_window / DAY);
  const days = z.string().check(
    z.refine((v) => /^\d+$/.test(v) && Number(v) >= minDays && Number(v) <= maxDays, {
      error: `需为 ${minDays}–${maxDays} 天`,
    }),
  );
  const minOrder = BigInt(limits.min_order);
  const cap = BigInt(limits.order_cap);
  const durations = new Set(durationOptions(limits.auction).map((d) => d.value));
  return z
    .object({
      title: z.string().check(
        z.refine((v) => v.trim().length >= 1 && v.trim().length <= 120, {
          error: "标题需为 1–120 个字",
        }),
      ),
      currency: z.enum(["USDT", "CNY"]),
      cnyRate: z.string(),
      start: z.string(),
      increment: z.string(),
      reserve: z.string(),
      duration: z.string().check(z.refine((v) => durations.has(v), { error: "请选择拍卖时长" })),
      eligibility: z.enum(["none", "10", "50", "100", "fixed"]),
      eligibilityAmount: z.string(),
      instant: z.boolean(),
      shipDays: z.string(),
      confirmDays: days,
      feePayer: z.enum(["0", "1", "2"]),
    })
    .check(
      z.refine((v) => v.currency !== "CNY" || parseCnyPerUsdt(v.cnyRate) !== null, {
        error: "汇率需在 5.0000–10.0000 之间，最多 4 位小数",
        path: ["cnyRate"],
      }),
      z.refine(
        (v) => {
          const p = auctionPrices(v);
          const usdt = pricesUsdt(p, p.start);
          return p.start !== null && (usdt === null || usdt >= minOrder);
        },
        {
          error: `起拍价需大于 0，折合不低于 ${formatUsdt(minOrder)} USDT`,
          path: ["start"],
        },
      ),
      z.refine(
        (v) => {
          const p = auctionPrices(v);
          if (p.increment === null) return false;
          if (p.start === null || (p.currency === "CNY" && p.rate === null)) return true;
          return p.increment >= minIncrement(p.currency, p.start, p.rate);
        },
        { error: "加价幅度不能低于 1 USDT 或起拍价的 1%（取大）", path: ["increment"] },
      ),
      z.refine(
        (v) => {
          const p = auctionPrices(v);
          return p.reserve !== null && (p.start === null || p.reserve >= p.start);
        },
        { error: "保留价不能低于起拍价", path: ["reserve"] },
      ),
      z.refine(
        (v) => {
          const p = auctionPrices(v);
          const usdt = pricesUsdt(p, p.reserve);
          return usdt === null || usdt <= cap;
        },
        {
          error: `保留价（不填时为起拍价）折合不能超过单笔上限 ${formatUsdt(cap)} USDT`,
          path: ["reserve"],
        },
      ),
      z.refine(
        (v) => {
          if (v.eligibility !== "fixed") return true;
          const a = parseUsdt(v.eligibilityAmount);
          return a !== null && a > 0n;
        },
        { error: "请填写余额门槛（USDT）", path: ["eligibilityAmount"] },
      ),
      z.refine((v) => v.instant || days.safeParse(v.shipDays).success, {
        error: `发货期限需为 ${minDays}–${maxDays} 天`,
        path: ["shipDays"],
      }),
    );
}

export type AuctionInput = z.infer<ReturnType<typeof auctionSchema>>;

/** 最低加价幅度的提示（如「至少 ¥7.20」）；价格还没填好时为 null。 */
export function minIncrementHint(v: AuctionInput): string | null {
  const p = auctionPrices(v);
  if (p.start === null || (p.currency === "CNY" && p.rate === null)) return null;
  return `至少 ${formatAuctionAmount(p.currency, minIncrement(p.currency, p.start, p.rate))}`;
}

function eligibility(v: AuctionInput): AuctionEligibility {
  if (v.eligibility === "none") return { kind: "none" };
  if (v.eligibility === "fixed") {
    return { kind: "fixed", usdt: String(parseUsdt(v.eligibilityAmount)) };
  }
  return { kind: "percent", percent: Number(v.eligibility) };
}

/** 校验通过的表单值 → 创建请求里的价格、条款部分（金额为最小单位字符串，天数转秒）。 */
export function auctionTerms(
  v: AuctionInput,
): Omit<NewAuctionRequest, "title" | "html" | "source" | "captcha"> {
  const p = auctionPrices(v);
  if (p.start === null || p.increment === null || p.reserve === null) {
    throw new Error("auctionTerms called with invalid input");
  }
  const confirm = Number(v.confirmDays) * DAY;
  return {
    currency: v.currency === "CNY" ? "cny" : "usdt",
    cny_per_usdt: p.rate,
    start_price: String(p.start),
    increment: String(p.increment),
    reserve_price: String(p.reserve),
    duration_secs: Number(v.duration),
    eligibility: eligibility(v),
    terms: {
      instant: v.instant,
      // 即时发货不用发货期限，合约仍要字段，填确认期同值（与交易帖一致）
      ship_window: v.instant ? confirm : Number(v.shipDays) * DAY,
      confirm_window: confirm,
      fee_payer: Number(v.feePayer),
    },
  };
}
