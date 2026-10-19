import type { Schemas } from "@akari/api-client";
import { describe, expect, it } from "vitest";
import { composeReason, reasonComplete } from "./reason";
import { topicActions } from "./topic-actions";

const flags = { pinned: false, featured: true, locked: false, hidden: false };
const mod = {
  roles: ["moderator"],
  boards: [3],
  suspended: false,
  site_permissions: [],
  board_permissions: ["topic_moderate", "topic_hide", "post_delete", "mute", "auction_ban"],
  admin_backend: false,
} as unknown as Schemas["StaffView"];

describe("topic moderation menu", () => {
  it("lists state-aware actions in own board only", () => {
    expect(topicActions(mod, 3, flags, true).map((a) => a.action)).toEqual([
      "pin",
      "unfeature",
      "lock",
      "move",
      "hide",
    ]);
    expect(topicActions(mod, 4, flags, true)).toEqual([]);
    expect(topicActions(null, 3, flags, true)).toEqual([]);
  });

  it("drops move without a permitted target and respects hide permission", () => {
    expect(topicActions(mod, 3, flags, false).map((a) => a.action)).not.toContain("move");
    const noHide = { ...mod, board_permissions: ["topic_moderate"] } as Schemas["StaffView"];
    expect(
      topicActions(noHide, 3, { ...flags, hidden: true }, true).map((a) => a.action),
    ).not.toContain("unhide");
  });
});

describe("reasons", () => {
  it("composes preset and note; 其他 needs a note", () => {
    expect(composeReason({ preset: "违反版规", note: "  " })).toBe("违反版规");
    expect(composeReason({ preset: "违反版规", note: " 第 3 条 " })).toBe("违反版规：第 3 条");
    expect(reasonComplete({ preset: "其他", note: "" })).toBe(false);
    expect(reasonComplete({ preset: "其他", note: "说明" })).toBe(true);
  });
});
