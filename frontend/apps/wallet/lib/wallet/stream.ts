import { claimPayout, streamUnlocked } from "@akari/fees";

// 分期担保（StreamEscrow）订单页的解锁进度与「开始 / 领取」规则（纯函数，与合约 start / claim 的条件一致）。
// 数据是链上 `getOrder` 的 latest 状态（indexer 不带净本金、上次领取时间与领取 gas 费）；时间一律 Unix 秒。

/** StreamEscrow.State */
export const StreamState = { None: 0, Funded: 1, Disputed: 2 } as const;

/** StreamEscrow.CLAIM_INTERVAL：两次领取至少间隔 24 小时（plan 第 5 节「领取每天 1 次」）。 */
export const CLAIM_INTERVAL = 86_400n;

/** 链上订单里这里用到的字段（`getOrder` 的返回值满足这个形状）。 */
export interface StreamOrder {
  state: number;
  startDeadline: bigint;
  startedAt: bigint;
  /** 每期秒数 */
  interval: number;
  periods: number;
  net: bigint;
  claimed: bigint;
  claimGas: bigint;
  lastClaimAt: bigint;
  disputedAt: bigint;
}

export interface StreamProgress {
  /** 已过期数（不超过总期数；争议后冻结在发起争议那一刻） */
  elapsed: number;
  unlocked: bigint;
  /** 剩余可退 = 净本金 − 已解锁 */
  refundable: bigint;
  /** 已解锁未领取 */
  claimable: bigint;
  /** 全部期数已到期 */
  ended: boolean;
}

function endsAt(o: StreamOrder): bigint {
  return o.startedAt + BigInt(o.interval) * BigInt(o.periods);
}

/** 与合约 `_unlocked` 相同：按 min(现在, 发起争议时) 计整期数，未开始为 0。 */
export function streamProgress(o: StreamOrder, now: bigint): StreamProgress {
  const started = o.startedAt !== 0n;
  const t = o.disputedAt !== 0n ? o.disputedAt : now;
  const periods = BigInt(o.periods);
  const passed = started && t > o.startedAt ? (t - o.startedAt) / BigInt(o.interval) : 0n;
  const elapsed = passed < periods ? passed : periods;
  const unlocked = started ? streamUnlocked(o.net, elapsed, periods) : 0n;
  return {
    elapsed: Number(elapsed),
    unlocked,
    refundable: o.net - unlocked,
    claimable: unlocked > o.claimed ? unlocked - o.claimed : 0n,
    ended: started && now >= endsAt(o),
  };
}

/** 卖家现在能否 `start`：已付款、未开始、没过开始期限。 */
export function canStart(o: StreamOrder, now: bigint): boolean {
  return o.state === StreamState.Funded && o.startedAt === 0n && now <= o.startDeadline;
}

export type ClaimCheck =
  /** `payout` 卖家这次实收；`settles` = 已到期，这次领取同时结清订单（不扣领取 gas 费） */
  | { ok: true; payout: bigint; settles: boolean }
  /** `retryAt` 冷却中时下次可领的时间 */
  | { ok: false; reason: string; retryAt?: bigint };

/**
 * 卖家现在能否 `claim`，与合约一致：服务进行中；到期后领取即结清（无冷却、无领取 gas 费）；
 * 否则距上次领取满 24 小时，且可领取 > 领取 gas 费（当次从中扣除）。
 */
export function claimCheck(o: StreamOrder, now: bigint): ClaimCheck {
  if (o.state !== StreamState.Funded) return { ok: false, reason: "订单不在服务期内" };
  if (o.startedAt === 0n) return { ok: false, reason: "还没有开始服务" };
  const p = streamProgress(o, now);
  if (p.ended) return { ok: true, payout: o.net - o.claimed, settles: true };
  if (o.lastClaimAt !== 0n && now < o.lastClaimAt + CLAIM_INTERVAL) {
    return {
      ok: false,
      reason: "距上次领取不足 24 小时",
      retryAt: o.lastClaimAt + CLAIM_INTERVAL,
    };
  }
  const payout = claimPayout(p.unlocked, o.claimed, o.claimGas);
  if (payout === null) return { ok: false, reason: "可领取金额还不够付领取 gas 费" };
  return { ok: true, payout, settles: false };
}

/** 每期时长 → 进度条标题。 */
export function periodLabel(intervalSecs: number): string {
  switch (intervalSecs) {
    case 3600:
      return "每小时解锁";
    case 86_400:
      return "每天解锁";
    case 7 * 86_400:
      return "每周解锁";
    case 30 * 86_400:
      return "每月解锁";
    default:
      return `每 ${Math.round(intervalSecs / 3600)} 小时解锁`;
  }
}
