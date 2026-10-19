// 后台会话期限（plan 第 9 节、ADR 0013 第 4 条）：闲置 2 小时、最长 12 小时；敏感操作要求 5 分钟内再验证过。
// 服务端每个后台请求都会顺延闲置期限，页面只看得到自己发出的请求，所以在本地按「最后一次成功请求」估计，
// 真正的判断永远在服务端：任何请求返回 401 就当作已过期。

import type { AdminSessionView } from "./api";

/** 与后端 `forum_api::admin::IDLE_SECS` 一致。 */
export const IDLE_SECS = 2 * 3600;
/** 到期前多久提醒。 */
export const WARN_SECS = 5 * 60;
/** 再验证剩余不足这么多秒时当作已失效，免得请求路上刚好过期。 */
const REVERIFY_MARGIN_SECS = 15;

/** 会话时钟，全部换算成本机时间（Unix 秒）。 */
export interface SessionClock {
  absoluteExpiresAt: number;
  idleExpiresAt: number;
  reverifiedUntil: number | null;
}

/**
 * 服务端时钟与本机的差（本机 + skew ≈ 服务端）。刚返回的会话视图里，闲置期限 = 服务端当前时间 + 2 小时，
 * 由此反推服务端时间；闲置期限被最长期限截断时推不出来，按 0 处理。
 */
export function serverSkew(view: AdminSessionView, localNow: number): number {
  if (view.expires_at >= view.absolute_expires_at) return 0;
  return view.expires_at - IDLE_SECS - localNow;
}

/** 由服务端返回的会话视图建立本地时钟。 */
export function clockFrom(view: AdminSessionView, localNow: number): SessionClock {
  const skew = serverSkew(view, localNow);
  return {
    absoluteExpiresAt: view.absolute_expires_at - skew,
    idleExpiresAt: view.expires_at - skew,
    reverifiedUntil: view.reverified_until == null ? null : view.reverified_until - skew,
  };
}

/** 一次成功的后台请求：服务端已把闲置期限顺延到现在 + 2 小时（不超过最长期限）。 */
export function touch(clock: SessionClock, localNow: number): SessionClock {
  return { ...clock, idleExpiresAt: Math.min(clock.absoluteExpiresAt, localNow + IDLE_SECS) };
}

export type SessionPhase =
  | { phase: "active" }
  | { phase: "warning" | "expired"; reason: "idle" | "absolute"; remaining: number };

/** 当前所处阶段：正常、快到期（还剩 5 分钟内）、已过期；`reason` 说明是闲置还是到了最长时长。 */
export function sessionPhase(clock: SessionClock, localNow: number): SessionPhase {
  const absolute = clock.absoluteExpiresAt <= clock.idleExpiresAt;
  const reason = absolute ? "absolute" : "idle";
  const remaining = Math.max(0, Math.min(clock.absoluteExpiresAt, clock.idleExpiresAt) - localNow);
  if (remaining <= 0) return { phase: "expired", reason, remaining: 0 };
  if (remaining <= WARN_SECS) return { phase: "warning", reason, remaining };
  return { phase: "active" };
}

/** 敏感操作前：5 分钟内用后台 Passkey 再验证过（留一点余量）。 */
export function reverifyFresh(clock: SessionClock, localNow: number): boolean {
  return clock.reverifiedUntil != null && clock.reverifiedUntil - localNow > REVERIFY_MARGIN_SECS;
}

/** 剩余时间 → 「4 分 05 秒」「1 小时 20 分」。 */
export function formatRemaining(secs: number): string {
  const s = Math.max(0, Math.floor(secs));
  if (s >= 3600) return `${Math.floor(s / 3600)} 小时 ${Math.floor((s % 3600) / 60)} 分`;
  return `${Math.floor(s / 60)} 分 ${String(s % 60).padStart(2, "0")} 秒`;
}
