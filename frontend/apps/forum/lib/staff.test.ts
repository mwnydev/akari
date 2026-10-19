import type { Me } from "@akari/api-client";
import { describe, expect, it } from "vitest";
import { can, durationChoices, durationLimit, isArbitrator, sanctionChoices } from "./staff";

type Staff = NonNullable<Me["staff"]>;

const MOD_PERMS = [
  "board_access",
  "topic_moderate",
  "topic_hide",
  "post_delete",
  "mute",
  "auction_ban",
  "view_sanctions",
] as const;

// 与后端 StaffView::from 对版主、管理员给出的值一致
const moderator: Staff = {
  roles: ["moderator"],
  boards: [3, 7],
  suspended: false,
  site_permissions: [],
  board_permissions: [...MOD_PERMS],
  max_mute_secs: 7 * 86_400,
  max_auction_ban_secs: 30 * 86_400,
  admin_backend: false,
};

const admin: Staff = {
  roles: ["admin"],
  boards: [],
  suspended: false,
  site_permissions: [...MOD_PERMS, "post_edit", "ban", "ip_ban", "device_ban"],
  board_permissions: [],
  max_mute_secs: null,
  max_auction_ban_secs: null,
  admin_backend: false,
};

describe("permission gating", () => {
  it("moderators act only in their own boards", () => {
    expect(can(moderator, "topic_hide", 3)).toBe(true);
    expect(can(moderator, "topic_hide", 4)).toBe(false);
    expect(can(moderator, "topic_hide")).toBe(false);
    expect(can(moderator, "ban", 3)).toBe(false);
    expect(can(moderator, "post_edit", 3)).toBe(false);
  });

  it("admins act everywhere; plain users and suspended staff nowhere", () => {
    expect(can(admin, "topic_hide", 4)).toBe(true);
    expect(can(admin, "ban")).toBe(true);
    expect(can(null, "topic_hide", 3)).toBe(false);
    expect(can({ ...admin, suspended: true }, "topic_hide", 3)).toBe(false);
  });

  it("only owners and admins arbitrate", () => {
    expect(isArbitrator(admin)).toBe(true);
    expect(isArbitrator({ ...moderator, roles: ["owner"] })).toBe(true);
    expect(isArbitrator(moderator)).toBe(false);
    expect(isArbitrator(null)).toBe(false);
  });
});

describe("sanction choices", () => {
  it("moderators get board-scoped mute and auction ban only", () => {
    expect(sanctionChoices(moderator, 3)).toEqual([
      { kind: "mute", site: false, board: true },
      { kind: "auction_ban", site: false, board: true },
    ]);
    expect(sanctionChoices(moderator, 4)).toEqual([]);
  });

  it("admins get every kind; ban/IP/device are site-wide", () => {
    const kinds = sanctionChoices(admin, 4);
    expect(kinds.map((k) => k.kind)).toEqual([
      "mute",
      "auction_ban",
      "ban",
      "ip_ban",
      "device_ban",
    ]);
    expect(kinds.find((k) => k.kind === "ban")).toEqual({ kind: "ban", site: true, board: false });
    expect(kinds.find((k) => k.kind === "mute")).toEqual({ kind: "mute", site: true, board: true });
  });

  it("duration options respect the per-role limit", () => {
    const mute = durationChoices(durationLimit(moderator, "mute")).map((d) => d.label);
    expect(mute).toEqual(["1 小时", "1 天", "3 天", "7 天"]);
    const auction = durationChoices(durationLimit(moderator, "auction_ban")).map((d) => d.label);
    expect(auction.at(-1)).toBe("30 天");
    expect(auction).not.toContain("永久");
    expect(durationChoices(durationLimit(admin, "mute")).at(-1)?.secs).toBeNull();
  });
});
