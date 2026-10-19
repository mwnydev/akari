import { describe, expect, it } from "vitest";
import {
  auctionSchema,
  auctionTerms,
  durationOptions,
  minIncrementHint,
  type AuctionInput,
} from "./auction-schema";

const auction = { settings: { min_duration_secs: 86_400, max_duration_secs: 7 * 86_400 } };

const limits = {
  min_order: "1000000",
  order_cap: "200000000",
  min_window: 86_400,
  max_window: 30 * 86_400,
  auction,
};

const valid: AuctionInput = {
  title: "绝版手办",
  currency: "USDT",
  cnyRate: "",
  start: "20",
  increment: "1",
  reserve: "",
  duration: String(3 * 86_400),
  eligibility: "none",
  eligibilityAmount: "",
  instant: false,
  shipDays: "3",
  confirmDays: "7",
  feePayer: "0",
};

function errors(v: Partial<AuctionInput>) {
  const r = auctionSchema(limits).safeParse({ ...valid, ...v });
  return r.success
    ? {}
    : Object.fromEntries(r.error.issues.map((i) => [i.path.join("."), i.message]));
}

describe("auction form", () => {
  it("offers only durations inside the owner's bounds", () => {
    expect(durationOptions(auction).map((d) => d.label)).toEqual([
      "1 天",
      "2 天",
      "3 天",
      "5 天",
      "7 天",
    ]);
    expect(errors({ duration: String(14 * 86_400) }).duration).toBe("请选择拍卖时长");
  });

  it("USDT: increment floor max(1 USDT, 1%), reserve ≥ start and within the order cap", () => {
    expect(errors({})).toEqual({});
    expect(errors({ increment: "0.99" }).increment).toMatch(/1 USDT 或起拍价的 1%/);
    expect(errors({ start: "150", increment: "1.49" }).increment).toBeDefined();
    expect(errors({ start: "150", increment: "1.5" }).increment).toBeUndefined();
    expect(errors({ reserve: "10" }).reserve).toBe("保留价不能低于起拍价");
    expect(errors({ reserve: "200.000001" }).reserve).toMatch(/单笔上限 200.00 USDT/);
    expect(errors({ start: "0.5" }).start).toMatch(/不低于 1.00 USDT/);
  });

  it("CNY: needs a rate in 5.0000–10.0000; limits compare the converted USDT", () => {
    const cny = { currency: "CNY" as const, start: "500", increment: "7.2" };
    expect(errors({ ...cny, cnyRate: "72" }).cnyRate).toMatch(/5.0000–10.0000/);
    expect(errors({ ...cny, cnyRate: "7.2" })).toEqual({});
    expect(errors({ ...cny, cnyRate: "7.2", increment: "7.19" }).increment).toBeDefined();
    // ¥1,440.01 at 7.2 → 200.000001 USDT，超过单笔上限
    expect(errors({ ...cny, cnyRate: "7.2", reserve: "1440.01" }).reserve).toMatch(/单笔上限/);
    expect(errors({ ...cny, cnyRate: "7.2", reserve: "1440" }).reserve).toBeUndefined();
    expect(minIncrementHint({ ...valid, ...cny, cnyRate: "7.2" })).toBe("至少 ¥7.20");
  });

  it("maps to the create request in smallest units; empty reserve = start price", () => {
    const t = auctionTerms({
      ...valid,
      currency: "CNY",
      cnyRate: "7.1234",
      start: "500",
      increment: "10",
      eligibility: "fixed",
      eligibilityAmount: "50",
      instant: true,
      feePayer: "1",
    });
    expect(t).toEqual({
      currency: "cny",
      cny_per_usdt: 71_234,
      start_price: "50000",
      increment: "1000",
      reserve_price: "50000",
      duration_secs: 3 * 86_400,
      eligibility: { kind: "fixed", usdt: "50000000" },
      terms: { instant: true, ship_window: 7 * 86_400, confirm_window: 7 * 86_400, fee_payer: 1 },
    });
    expect(auctionTerms({ ...valid, eligibility: "50" }).eligibility).toEqual({
      kind: "percent",
      percent: 50,
    });
    expect(errors({ eligibility: "fixed" }).eligibilityAmount).toBeDefined();
  });
});
