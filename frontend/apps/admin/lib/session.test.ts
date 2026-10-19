import { describe, expect, it } from "vitest";
import type { AdminSessionView } from "./api";
import {
  IDLE_SECS,
  clockFrom,
  formatRemaining,
  reverifyFresh,
  serverSkew,
  sessionPhase,
  touch,
} from "./session";

const T = 1_800_000_000;

/** 服务端在 `serverNow` 返回的会话视图：会话建立于 `createdAt`。 */
function view(serverNow: number, createdAt: number, reverifiedAt: number | null): AdminSessionView {
  const absolute = createdAt + 12 * 3600;
  return {
    user_id: 1,
    absolute_expires_at: absolute,
    expires_at: Math.min(absolute, serverNow + IDLE_SECS),
    reverified_until:
      reverifiedAt != null && reverifiedAt + 300 >= serverNow ? reverifiedAt + 300 : null,
  };
}

describe("session clock", () => {
  it("infers server skew from the idle deadline", () => {
    // 服务端比本机快 40 秒
    expect(serverSkew(view(T + 40, T, T), T)).toBe(40);
    // 闲置期限被最长期限截断时推不出来
    expect(serverSkew(view(T + 11.5 * 3600, T, null), T)).toBe(0);
  });

  it("converts deadlines to local time", () => {
    const c = clockFrom(view(T + 40, T, T + 40), T);
    expect(c.idleExpiresAt).toBe(T + IDLE_SECS);
    expect(c.absoluteExpiresAt).toBe(T - 40 + 12 * 3600);
    expect(c.reverifiedUntil).toBe(T + 300);
  });

  it("successful requests extend the idle deadline up to the absolute limit", () => {
    const c = clockFrom(view(T, T, T), T);
    expect(touch(c, T + 3600).idleExpiresAt).toBe(T + 3600 + IDLE_SECS);
    expect(touch(c, T + 11 * 3600).idleExpiresAt).toBe(T + 12 * 3600);
  });

  it("warns 5 minutes before expiry and tells idle from absolute", () => {
    const c = clockFrom(view(T, T, null), T);
    expect(sessionPhase(c, T + 60).phase).toBe("active");
    expect(sessionPhase(c, T + IDLE_SECS - 120)).toEqual({
      phase: "warning",
      reason: "idle",
      remaining: 120,
    });
    expect(sessionPhase(c, T + IDLE_SECS)).toEqual({
      phase: "expired",
      reason: "idle",
      remaining: 0,
    });
    // 一直在用：最后受最长 12 小时限制
    const late = touch(c, T + 12 * 3600 - 600);
    expect(sessionPhase(late, T + 12 * 3600 - 200)).toMatchObject({
      phase: "warning",
      reason: "absolute",
    });
    expect(sessionPhase(late, T + 12 * 3600 + 1)).toMatchObject({
      phase: "expired",
      reason: "absolute",
    });
  });

  it("re-verification is valid for 5 minutes minus a margin", () => {
    const c = clockFrom(view(T, T - 60, T), T);
    expect(reverifyFresh(c, T + 200)).toBe(true);
    expect(reverifyFresh(c, T + 290)).toBe(false);
    expect(reverifyFresh(clockFrom(view(T, T - 3600, T - 600), T), T)).toBe(false);
  });

  it("formats remaining time", () => {
    expect(formatRemaining(245)).toBe("4 分 05 秒");
    expect(formatRemaining(4800)).toBe("1 小时 20 分");
    expect(formatRemaining(-3)).toBe("0 分 00 秒");
  });
});
