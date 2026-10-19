import type { Schemas } from "@akari/api-client";
import { describe, expect, it } from "vitest";
import {
  allocate,
  bpsLabel,
  disputedPrincipal,
  intentMatches,
  orderBond,
  parseIntent,
  percentToBps,
} from "./ruling";

describe("ratio → amount preview", () => {
  it("rounds like the contract: seller share floors, the rest goes to the buyer", () => {
    // 与后端 forum_core::arbitration 的测试同一组数
    expect(allocate(250_000_001n, 100_000_000n, 3333, 5000)).toEqual({
      principalToBuyer: 83_325_001n,
      principalToSeller: 166_675_000n,
      bondToBuyer: 50_000_000n,
    });
    expect(allocate(45_000_000n, 10_000_000n, 10_000, 2500)).toEqual({
      principalToBuyer: 45_000_000n,
      principalToSeller: 0n,
      bondToBuyer: 2_500_000n,
    });
    expect(allocate(45_000_000n, 0n, 0, 0).principalToSeller).toBe(45_000_000n);
    expect(allocate(3n, 7n, 3333, 3333)).toEqual({
      principalToBuyer: 1n,
      principalToSeller: 2n,
      bondToBuyer: 2n,
    });
  });

  it("knows the disputed principal only for one-time orders", () => {
    const order = { mode: "escrow", amount: "45000000", fees: { bond: "5000000" } };
    expect(disputedPrincipal(order as Schemas["OrderView"])).toBe(45_000_000n);
    expect(orderBond(order as Schemas["OrderView"])).toBe(5_000_000n);
    expect(disputedPrincipal({ ...order, mode: "stream" } as Schemas["OrderView"])).toBeNull();
    expect(orderBond({ ...order, fees: {} } as Schemas["OrderView"])).toBe(0n);
  });

  it("parses percentages into bps", () => {
    expect(percentToBps("100")).toBe(10_000);
    expect(percentToBps("62.5")).toBe(6250);
    expect(percentToBps("0.01")).toBe(1);
    expect(percentToBps("100.01")).toBeNull();
    expect(percentToBps("1.234")).toBeNull();
    expect(percentToBps("-1")).toBeNull();
    expect(bpsLabel(6250)).toBe("62.5%");
    expect(bpsLabel(10_000)).toBe("100%");
  });
});

describe("signed intent", () => {
  const json = JSON.stringify({
    v: 1,
    case_id: 9,
    kind: "panel",
    chain_id: 421614,
    contract: "0x" + "44".repeat(20),
    chain_order_id: 7,
    to_buyer_bps: 7500,
    bond_to_buyer_bps: 0,
    principal_to_buyer: "33750000",
    principal_to_seller: "11250000",
    bond_to_buyer: "0",
    reason_sha256: "0x" + "00".repeat(32),
    nonce: "0x" + "11".repeat(32),
  });

  it("extracts the amounts the passkey will sign", () => {
    const i = parseIntent(json);
    expect(i).toMatchObject({ caseId: 9, principalToBuyer: "33750000", toBuyerBps: 7500 });
    expect(
      i && intentMatches(i, { caseId: 9, kind: "panel", toBuyerBps: 7500, bondToBuyerBps: 0 }),
    ).toBe(true);
    expect(
      i && intentMatches(i, { caseId: 9, kind: "panel", toBuyerBps: 7000, bondToBuyerBps: 0 }),
    ).toBe(false);
  });

  it("rejects malformed intents", () => {
    expect(parseIntent("not json")).toBeNull();
    expect(parseIntent(JSON.stringify({ case_id: 9 }))).toBeNull();
    expect(parseIntent(json.replace('"33750000"', '"-1"'))).toBeNull();
  });
});
