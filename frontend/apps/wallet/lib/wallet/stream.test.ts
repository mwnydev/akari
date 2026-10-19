import { claimPayout, orderCost, streamNet, streamUnlocked, type FeePayer } from "@akari/fees";
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import type { OrderView } from "./api";
import { streamAction } from "./order";
import {
  CLAIM_INTERVAL,
  StreamState,
  canStart,
  claimCheck,
  periodLabel,
  streamProgress,
  type StreamOrder,
} from "./stream";

// 分期进度与领取条件：解锁额与 @akari/fees（合约、Rust 共用同一份 test-vectors/fees.json）逐点一致。

const DAY = 86_400n;
const T0 = 1_800_000_000n;

const order = (o: Partial<StreamOrder> = {}): StreamOrder => ({
  state: StreamState.Funded,
  startDeadline: T0 + DAY,
  startedAt: T0,
  interval: 86_400,
  periods: 30,
  net: 298_900_000n,
  claimed: 0n,
  claimGas: 20_000n,
  lastClaimAt: 0n,
  disputedAt: 0n,
  ...o,
});

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

const vectors = JSON.parse(
  readFileSync(new URL("../../../../../test-vectors/fees.json", import.meta.url), "utf8"),
) as { claimGas: string; streams: StreamVector[] };

describe("分期解锁进度", () => {
  it.each(vectors.streams.map((v) => [v.name, v] as const))("共享测试向量：%s", (_n, v) => {
    const c = orderCost({
      amount: BigInt(v.amount),
      feePayer: v.feePayer,
      feeBps: BigInt(v.feeBps),
      minFee: BigInt(v.minFee),
      campaign: false,
      baseGas: BigInt(v.baseGas),
      disputeMulBps: 10_000n,
    });
    const net = streamNet(c.fee.seller, c.gas.seller, BigInt(v.amount));
    expect(net.toString()).toBe(v.expect.unlockableNet);
    for (const p of v.expect.points) {
      // 已过 elapsedPeriods 期再多 1 秒：整期数向下取整
      const now = T0 + BigInt(p.elapsedPeriods) * DAY + 1n;
      const got = streamProgress(order({ net, periods: v.periods }), now);
      expect(got.unlocked.toString()).toBe(p.unlocked);
      expect(got.elapsed).toBe(Math.min(p.elapsedPeriods, v.periods));
    }
  });

  it("可领取 = 已解锁 − 已领取，剩余可退 = 净本金 − 已解锁", () => {
    const o = order({ claimed: streamUnlocked(298_900_000n, 11n, 30n) });
    const p = streamProgress(o, T0 + 12n * DAY);
    expect(p.unlocked).toBe(streamUnlocked(o.net, 12n, 30n));
    expect(p.claimable).toBe(p.unlocked - o.claimed);
    expect(p.refundable).toBe(o.net - p.unlocked);
    expect(p.ended).toBe(false);
  });

  it("未开始全为 0；争议后按发起争议时冻结；到期封顶", () => {
    expect(streamProgress(order({ startedAt: 0n }), T0 + 5n * DAY).unlocked).toBe(0n);
    const disputed = order({ state: StreamState.Disputed, disputedAt: T0 + 3n * DAY + 5n });
    expect(streamProgress(disputed, T0 + 20n * DAY).elapsed).toBe(3);
    const end = streamProgress(order(), T0 + 99n * DAY);
    expect(end).toMatchObject({ elapsed: 30, unlocked: 298_900_000n, refundable: 0n, ended: true });
  });
});

describe("开始与领取条件（与合约 start / claim 一致）", () => {
  it("开始期限内、未开始才能 start", () => {
    const fresh = order({ startedAt: 0n });
    expect(canStart(fresh, T0 + DAY)).toBe(true);
    expect(canStart(fresh, T0 + DAY + 1n)).toBe(false);
    expect(canStart(order(), T0)).toBe(false);
    expect(canStart({ ...fresh, state: StreamState.Disputed }, T0)).toBe(false);
  });

  it("可领取 > 领取 gas 费才能领，实收与 claimPayout 一致", () => {
    const now = T0 + 2n * DAY;
    const r = claimCheck(order(), now);
    const unlocked = streamUnlocked(298_900_000n, 2n, 30n);
    expect(r).toEqual({ ok: true, payout: claimPayout(unlocked, 0n, 20_000n), settles: false });
    // 刚好等于 gas 费：不能领
    const tiny = order({ net: 600_000n, claimGas: 20_000n });
    expect(claimCheck(tiny, T0 + DAY)).toMatchObject({ ok: false });
    expect(claimPayout(20_000n, 0n, 20_000n)).toBeNull();
  });

  it("两次领取至少间隔 24 小时，给出下次可领时间", () => {
    const last = T0 + 3n * DAY;
    const o = order({ lastClaimAt: last, claimed: streamUnlocked(298_900_000n, 3n, 30n) });
    expect(claimCheck(o, last + CLAIM_INTERVAL - 1n)).toEqual({
      ok: false,
      reason: "距上次领取不足 24 小时",
      retryAt: last + CLAIM_INTERVAL,
    });
    expect(claimCheck(o, last + CLAIM_INTERVAL + DAY).ok).toBe(true);
  });

  it("全部到期后领取即结清：不看冷却、不扣领取 gas 费", () => {
    const o = order({ claimed: 100_000_000n, lastClaimAt: T0 + 30n * DAY - 10n });
    expect(claimCheck(o, T0 + 30n * DAY)).toEqual({
      ok: true,
      payout: 298_900_000n - 100_000_000n,
      settles: true,
    });
  });

  it("未开始、争议中不能领", () => {
    expect(claimCheck(order({ startedAt: 0n }), T0).ok).toBe(false);
    expect(claimCheck(order({ state: StreamState.Disputed }), T0 + 5n * DAY).ok).toBe(false);
  });

  it("期长 → 进度条标题", () => {
    expect(periodLabel(3600)).toBe("每小时解锁");
    expect(periodLabel(86_400)).toBe("每天解锁");
    expect(periodLabel(30 * 86_400)).toBe("每月解锁");
  });
});

describe("分期订单页按钮", () => {
  const STREAM = "0x6022875397e11A32BbAEB91d80258CBF4AA6F4A8";
  const SELLER = "0x2222222222222222222222222222222222222222";
  const BUYER = "0x1111111111111111111111111111111111111111";
  const view = { mode: "stream", contract: STREAM, seller: { address: SELLER } } as OrderView;
  const at = (secs: bigint) => Number(secs) * 1000;

  it("卖家：未开始给「开始服务」，进行中给「领取」（条件不满足时带原因）", () => {
    const fresh = order({ startedAt: 0n });
    expect(streamAction(view, fresh, SELLER, at(T0), STREAM)?.kind).toBe("start");
    const claim = streamAction(view, order(), SELLER, at(T0 + 2n * DAY), STREAM);
    expect(claim).toMatchObject({ kind: "claim", label: "领取已解锁部分", claim: { ok: true } });
    const early = streamAction(view, order(), SELLER, at(T0 + 10n), STREAM);
    expect(early).toMatchObject({ kind: "claim", claim: { ok: false } });
    const end = streamAction(view, order(), SELLER, at(T0 + 40n * DAY), STREAM);
    expect(end?.label).toBe("领取剩余并结清");
  });

  it("买家只看进度；别的合约、还没读到链上状态、开始期限已过都不给按钮", () => {
    expect(streamAction(view, order(), BUYER, at(T0 + 2n * DAY), STREAM)).toBeNull();
    expect(streamAction(view, order(), SELLER, at(T0), `0x${"9".repeat(40)}`)).toBeNull();
    expect(streamAction(view, undefined, SELLER, at(T0), STREAM)).toBeNull();
    expect(
      streamAction(view, order({ startedAt: 0n }), SELLER, at(T0 + 2n * DAY), STREAM),
    ).toBeNull();
  });
});
