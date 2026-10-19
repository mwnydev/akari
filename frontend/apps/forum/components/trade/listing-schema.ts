import type { Schemas } from "@akari/api-client";
import { formatUsdt, parseUsdt } from "@akari/ui";
import * as z from "zod/mini";
import { parseCnyPerUsdt } from "../../lib/fees";

const DAY = 86_400;

export type TradeLimits = Pick<
  Schemas["TradeParams"],
  "min_order" | "order_cap" | "min_window" | "max_window"
>;

/** 发起交易帖表单的校验（与后端 check_windows / check_bond / 金额上下限一致，提前在浏览器里报错）。 */
export function listingSchema(limits: TradeLimits) {
  const minDays = Math.ceil(limits.min_window / DAY);
  const maxDays = Math.floor(limits.max_window / DAY);
  const days = z.string().check(
    z.refine((v) => /^\d+$/.test(v) && Number(v) >= minDays && Number(v) <= maxDays, {
      error: `需为 ${minDays}–${maxDays} 天`,
    }),
  );
  const min = BigInt(limits.min_order);
  const cap = BigInt(limits.order_cap);
  return z
    .object({
      title: z.string().check(
        z.refine((v) => v.trim().length >= 1 && v.trim().length <= 120, {
          error: "标题需为 1–120 个字",
        }),
      ),
      price: z.string().check(
        z.refine(
          (v) => {
            const p = parseUsdt(v);
            return p !== null && p >= min && p <= cap;
          },
          { error: `价格需在 ${formatUsdt(min)}–${formatUsdt(cap)} USDT 之间，最多 6 位小数` },
        ),
      ),
      instant: z.boolean(),
      shipDays: z.string(),
      confirmDays: days,
      feePayer: z.enum(["0", "1", "2"]),
      bond: z
        .string()
        .check(z.refine((v) => parseUsdt(v || "0") !== null, { error: "保证金格式不正确" })),
      // 选填的人民币参考汇率（ADR 0011），只用于显示参考价
      cnyRate: z.string().check(
        z.refine((v) => v.trim() === "" || parseCnyPerUsdt(v) !== null, {
          error: "参考汇率需在 5.0000–10.0000 之间，最多 4 位小数",
        }),
      ),
    })
    .check(
      z.refine((v) => v.instant || days.safeParse(v.shipDays).success, {
        error: `发货期限需为 ${minDays}–${maxDays} 天`,
        path: ["shipDays"],
      }),
      z.refine(
        (v) => {
          const price = parseUsdt(v.price);
          const bond = parseUsdt(v.bond || "0");
          return price === null || bond === null || bond <= price;
        },
        { error: "保证金不能超过价格", path: ["bond"] },
      ),
    );
}

export type ListingInput = z.infer<ReturnType<typeof listingSchema>>;

/** 表单值 → POST /forums/{slug}/listings 的价格、条款与参考汇率（金额转成最小单位字符串，天数转秒）。 */
export function listingTerms(
  v: ListingInput,
): Pick<Schemas["NewListingRequest"], "price" | "terms" | "cny_per_usdt"> {
  const confirm = Number(v.confirmDays) * DAY;
  const rate = v.cnyRate.trim() === "" ? null : parseCnyPerUsdt(v.cnyRate);
  return {
    price: String(parseUsdt(v.price)),
    cny_per_usdt: rate === null ? null : Number(rate),
    terms: {
      instant: v.instant,
      // 即时发货不用发货期限，后端仍要求字段存在，填确认期同值即可
      ship_window: v.instant ? confirm : Number(v.shipDays) * DAY,
      confirm_window: confirm,
      fee_payer: Number(v.feePayer),
      bond: String(parseUsdt(v.bond || "0")),
      late_slash_bps: 0,
    },
  };
}
