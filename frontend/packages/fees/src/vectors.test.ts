import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import {
  type FeePayer,
  claimPayout,
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

interface Vectors {
  version: number;
  claimGas: string;
  fees: FeeVector[];
  streams: StreamVector[];
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
