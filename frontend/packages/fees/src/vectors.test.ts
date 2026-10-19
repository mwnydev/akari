import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import {
  type FeePayer,
  MAX_CNY_PER_USDT,
  MIN_CNY_PER_USDT,
  claimPayout,
  cnyFenToUsdt,
  isValidCnyPerUsdt,
  usdtToCnyFen,
  orderCost,
  split,
  feeTotal,
  streamNet,
  streamUnlocked,
} from "./index";

interface FeeVector {
  name: string;
  amount: string;
  feePayer: FeePayer;
  feeBps: number;
  minFee: string;
  campaign: boolean;
  baseGas: string;
  disputeMulBps: number;
  expect: Record<
    "buyerFee" | "sellerFee" | "buyerGas" | "sellerGas" | "buyerPays" | "disputeFee",
    string
  >;
}

interface StreamVector {
  name: string;
  amount: string;
  feePayer: FeePayer;
  feeBps: number;
  minFee: string;
  baseGas: string;
  periods: number;
  expect: { unlockableNet: string; points: { elapsedPeriods: number; unlocked: string }[] };
}

interface CnyVectors {
  minCnyPerUsdt: number;
  maxCnyPerUsdt: number;
  toUsdt: { name: string; priceFen: string; cnyPerUsdt: number; expect: string }[];
  reference: { name: string; usdt: string; cnyPerUsdt: number; expectFen: string }[];
}

interface Vectors {
  version: number;
  claimGas: string;
  fees: FeeVector[];
  streams: StreamVector[];
  cny: CnyVectors;
}

const vectors = JSON.parse(
  readFileSync(new URL("../../../../test-vectors/fees.json", import.meta.url), "utf8"),
) as Vectors;

describe("fee vectors", () => {
  it.each(vectors.fees.map((v) => [v.name, v] as const))("%s", (_name, v) => {
    const c = orderCost({
      amount: BigInt(v.amount),
      feePayer: v.feePayer,
      feeBps: BigInt(v.feeBps),
      minFee: BigInt(v.minFee),
      campaign: v.campaign,
      baseGas: BigInt(v.baseGas),
      disputeMulBps: BigInt(v.disputeMulBps),
    });
    expect(c.fee.buyer).toBe(BigInt(v.expect.buyerFee));
    expect(c.fee.seller).toBe(BigInt(v.expect.sellerFee));
    expect(c.gas.buyer).toBe(BigInt(v.expect.buyerGas));
    expect(c.gas.seller).toBe(BigInt(v.expect.sellerGas));
    expect(c.buyerPays).toBe(BigInt(v.expect.buyerPays));
    expect(c.disputeFee).toBe(BigInt(v.expect.disputeFee));
  });
});

describe("stream vectors", () => {
  it.each(vectors.streams.map((v) => [v.name, v] as const))("%s", (_name, v) => {
    const amount = BigInt(v.amount);
    const fee = split(feeTotal(amount, BigInt(v.feeBps), BigInt(v.minFee), false), v.feePayer);
    const gas = split(BigInt(v.baseGas), v.feePayer);
    const net = streamNet(fee.seller, gas.seller, amount);
    expect(net).toBe(BigInt(v.expect.unlockableNet));
    for (const p of v.expect.points) {
      expect(streamUnlocked(net, BigInt(p.elapsedPeriods), BigInt(v.periods))).toBe(
        BigInt(p.unlocked),
      );
    }
  });
});

describe("claim", () => {
  const claimGas = BigInt(vectors.claimGas);
  it("requires claimable strictly greater than claim gas", () => {
    expect(claimPayout(claimGas, 0n, claimGas)).toBeNull();
    expect(claimPayout(claimGas + 1n, 0n, claimGas)).toBe(1n);
    expect(claimPayout(10_000_000n, 4_000_000n, claimGas)).toBe(6_000_000n - claimGas);
  });
});

describe("cny vectors", () => {
  const { cny } = vectors;
  it("rate bounds match the contract", () => {
    expect(MIN_CNY_PER_USDT).toBe(BigInt(cny.minCnyPerUsdt));
    expect(MAX_CNY_PER_USDT).toBe(BigInt(cny.maxCnyPerUsdt));
    for (const r of [0n, 72n, 49_999n, 100_001n]) {
      expect(isValidCnyPerUsdt(r)).toBe(false);
      expect(() => cnyFenToUsdt(100n, r)).toThrow(RangeError);
      expect(() => usdtToCnyFen(1_000_000n, r)).toThrow(RangeError);
    }
  });
  it.each(cny.toUsdt.map((v) => [v.name, v] as const))("to USDT: %s", (_name, v) => {
    expect(cnyFenToUsdt(BigInt(v.priceFen), BigInt(v.cnyPerUsdt))).toBe(BigInt(v.expect));
  });
  it.each(cny.reference.map((v) => [v.name, v] as const))("reference: %s", (_name, v) => {
    expect(usdtToCnyFen(BigInt(v.usdt), BigInt(v.cnyPerUsdt))).toBe(BigInt(v.expectFen));
  });
});
