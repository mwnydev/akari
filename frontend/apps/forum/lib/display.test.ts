import { describe, expect, it } from "vitest";
import {
  cardLookup,
  filterMentionable,
  formatDays,
  formatTime,
  profileHref,
  userName,
  type UserCardView,
} from "./display";

describe("display helpers", () => {
  it("names users without a username by id", () => {
    expect(userName({ id: 7, username: null })).toBe("用户 7");
    expect(userName({ id: 7, username: "小林" })).toBe("小林");
    expect(profileHref({ id: 7, username: null })).toBe("/u/7");
    expect(profileHref({ id: 7, username: "小林" })).toBe("/u/%E5%B0%8F%E6%9E%97");
  });

  it("falls back to a placeholder card for users missing from the response", () => {
    const user = cardLookup([{ id: 1, username: "a", level: 3 }]);
    expect(user(1).level).toBe(3);
    expect(user(2)).toEqual({ id: 2, username: null, level: 1 });
  });

  it("formats times in the site time zone and windows in days", () => {
    expect(formatTime("2026-10-18T17:04:00Z")).toBe("2026-10-19 01:04");
    expect(formatDays(3 * 86_400)).toBe("3 天");
    expect(formatDays(36 * 3600)).toBe("36 小时");
  });

  it("filters mention candidates case-insensitively and caps the list", () => {
    const users: UserCardView[] = Array.from({ length: 12 }, (_, i) => ({
      id: i + 1,
      username: null,
      level: 1,
    }));
    users.push({ id: 99, username: "Momo", level: 1 });
    expect(filterMentionable(users, "mo").map((u) => u.id)).toEqual([99]);
    expect(filterMentionable(users, "")).toHaveLength(8);
    expect(filterMentionable(users, "用户 1").map((u) => u.id)).toEqual([1, 10, 11, 12]);
  });
});
