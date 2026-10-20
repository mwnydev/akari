import { describe, expect, it } from "vitest";
import {
  NO_REPUTATION,
  cardLookup,
  formatActivity,
  formatCount,
  formatDays,
  formatShortTime,
  formatTime,
  profileHref,
  tradeReputation,
  userName,
} from "./display";

describe("display helpers", () => {
  it("names users without a username by id", () => {
    expect(userName({ id: 7, username: null })).toBe("用户 7");
    expect(userName({ id: 7, username: "小林" })).toBe("小林");
    expect(profileHref({ id: 7, username: null })).toBe("/u/7");
    expect(profileHref({ id: 7, username: "小林" })).toBe("/u/%E5%B0%8F%E6%9E%97");
  });

  it("falls back to a placeholder card for users missing from the response", () => {
    const user = cardLookup([{ id: 1, username: "a", level: 3, reputation: NO_REPUTATION }]);
    expect(user(1).level).toBe(3);
    expect(user(2)).toEqual({ id: 2, username: null, level: 1, reputation: NO_REPUTATION });
  });

  it("formats times in the site time zone and windows in days", () => {
    expect(formatTime("2026-10-18T17:04:00Z")).toBe("2026-10-19 01:04");
    expect(formatDays(3 * 86_400)).toBe("3 天");
    expect(formatDays(36 * 3600)).toBe("36 小时");
  });

  it("drops the year in chat times for this year only", () => {
    const now = new Date("2026-12-01T00:00:00Z");
    expect(formatShortTime("2026-10-18T17:04:00Z", now)).toBe("10-19 01:04");
    expect(formatShortTime("2025-10-18T17:04:00Z", now)).toBe("2025-10-19 01:04");
  });

  it("converts reputation basis points to the card's permille", () => {
    expect(
      tradeReputation({
        success: 12,
        failed: 1,
        settled: 0,
        disputes_lost: 1,
        success_rate_bps: 9231,
        auction_defaults: 0,
        auction_banned_until: null,
      }),
    ).toEqual({ successPermille: 923, deals: 12, disputesLost: 1 });
    expect(tradeReputation(NO_REPUTATION)).toEqual({
      successPermille: 0,
      deals: 0,
      disputesLost: 0,
    });
  });

  it("abbreviates counts and recent activity", () => {
    expect(formatCount(9_999)).toBe("9999");
    expect(formatCount(12_840)).toBe("1.3万");
    const now = new Date("2026-10-20T08:00:00Z");
    expect(formatActivity("2026-10-20T07:59:30Z", now)).toBe("刚刚");
    expect(formatActivity("2026-10-20T07:55:00Z", now)).toBe("5 分钟");
    expect(formatActivity("2026-10-20T05:00:00Z", now)).toBe("3 小时");
    expect(formatActivity("2026-10-18T08:00:00Z", now)).toBe("2 天");
    expect(formatActivity("2026-08-01T00:00:00Z", now)).toBe("08-01");
    expect(formatActivity("2025-08-01T00:00:00Z", now)).toBe("2025-08-01");
  });
});
