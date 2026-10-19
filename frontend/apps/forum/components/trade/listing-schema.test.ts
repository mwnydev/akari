import { describe, expect, it } from "vitest";
import { listingSchema, listingTerms, type ListingInput } from "./listing-schema";

const limits = {
  min_order: "5000000",
  order_cap: "200000000",
  min_window: 86_400,
  max_window: 30 * 86_400,
};
const schema = listingSchema(limits);
const valid: ListingInput = {
  title: "JetBrains 全家桶一年授权",
  price: "45",
  instant: false,
  shipDays: "3",
  confirmDays: "7",
  feePayer: "0",
  bond: "",
  cnyRate: "",
};

function errors(v: Partial<ListingInput>): string[] {
  const r = schema.safeParse({ ...valid, ...v });
  return r.success ? [] : r.error.issues.map((i) => `${i.path.join(".")}: ${i.message}`);
}

describe("listingSchema", () => {
  it("accepts a normal listing and converts it to API terms", () => {
    expect(errors({})).toEqual([]);
    expect(listingTerms(valid)).toEqual({
      price: "45000000",
      cny_per_usdt: null,
      terms: {
        instant: false,
        ship_window: 3 * 86_400,
        confirm_window: 7 * 86_400,
        fee_payer: 0,
        bond: "0",
        late_slash_bps: 0,
      },
    });
  });

  it("enforces the on-chain price range and 6 decimals", () => {
    expect(errors({ price: "4.99" })[0]).toMatch(/^price: 价格需在 5.00–200.00 USDT/);
    expect(errors({ price: "200.000001" })).toHaveLength(1);
    expect(errors({ price: "10.1234567" })).toHaveLength(1);
  });

  it("checks windows, skipping the ship window for instant delivery", () => {
    expect(errors({ shipDays: "0" })).toEqual(["shipDays: 发货期限需为 1–30 天"]);
    expect(errors({ shipDays: "0", instant: true })).toEqual([]);
    expect(errors({ confirmDays: "31" })).toEqual(["confirmDays: 需为 1–30 天"]);
    expect(listingTerms({ ...valid, instant: true }).terms.ship_window).toBe(7 * 86_400);
  });

  it("accepts an optional CNY reference rate within the contract bounds", () => {
    expect(listingTerms({ ...valid, cnyRate: "7.2" }).cny_per_usdt).toBe(72_000);
    expect(errors({ cnyRate: " " })).toEqual([]);
    expect(errors({ cnyRate: "72" })).toEqual([
      "cnyRate: 参考汇率需在 5.0000–10.0000 之间，最多 4 位小数",
    ]);
    expect(errors({ cnyRate: "7.12345" })).toHaveLength(1);
  });

  it("rejects a bond above the price", () => {
    expect(errors({ bond: "45.000001" })).toEqual(["bond: 保证金不能超过价格"]);
    expect(listingTerms({ ...valid, bond: "10" }).terms.bond).toBe("10000000");
  });
});
