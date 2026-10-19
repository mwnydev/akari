import type { Schemas } from "@akari/api-client";
import { describe, expect, it } from "vitest";
import { boardColor } from "./board-color";
import { cardLookup, NO_REPUTATION } from "./display";
import { hotOrder, personCard, topicLink, topicRowView, tradeFacts } from "./topic-views";

const summary: Schemas["TopicSummary"] = {
  id: 7,
  forum_id: 2,
  author_id: 1,
  title: "标题",
  kind: 0,
  created_at: "2026-10-20T00:00:00Z",
  last_post_at: "2026-10-20T07:00:00Z",
  last_poster_id: null,
  last_floor: 3,
  replies: 2,
  views: 40,
  participants: [1, 3],
  trade: null,
  flags: { pinned: true, featured: false, locked: false, hidden: false },
};

const forum: Schemas["ForumView"] = {
  id: 2,
  parent_id: null,
  slug: "trade",
  name: "交易区",
  description: "",
  position: 3,
  view_min_level: 0,
  post_min_level: 1,
  reply_min_level: 1,
  allow_trade: true,
};

const traded = { ...NO_REPUTATION, success: 12, success_rate_bps: 9900 };
const user = cardLookup([{ id: 1, username: "卖家", level: 4, reputation: traded }]);

describe("topic views", () => {
  it("links trade topics with a listing to the trade page", () => {
    expect(topicLink(summary)).toBe("/t/7");
    const trade = { listing_id: 5, price: "45000000", instant: true, cny_per_usdt: null };
    expect(topicLink({ ...summary, kind: 1, trade })).toBe("/trade/5");
  });

  it("shows reputation only for people who have traded", () => {
    expect(personCard(user(1)).reputation).toEqual({
      successPermille: 990,
      deals: 12,
      disputesLost: 0,
    });
    expect(personCard(user(2))).toMatchObject({
      name: "用户 2",
      href: "/u/2",
      reputation: undefined,
    });
  });

  it("maps a summary to a list row", () => {
    const row = topicRowView(summary, user, { forum, now: new Date("2026-10-20T08:00:00Z") });
    expect(row).toMatchObject({
      href: "/t/7",
      kind: "normal",
      board: { name: "交易区", color: boardColor(forum), href: "/f/trade" },
      activity: "1 小时",
      pinned: true,
      participants: [{ name: "卖家" }, { name: "用户 3" }],
    });
  });

  it("summarises trade topics with a price", () => {
    expect(tradeFacts(summary, user)).toBeNull();
    const trade = { listing_id: 5, price: "45000000", instant: false, stock: 3 };
    expect(tradeFacts({ ...summary, kind: 1, trade }, user)).toEqual({
      price: "45000000",
      cnyPerUsdt: undefined,
      instant: false,
      stock: 3,
      // 卖家就是楼主：成功率取楼主名片上的信誉
      seller: { successPermille: 990, deals: 12, disputesLost: 0 },
    });
  });

  it("orders the hot tab by replies, then views", () => {
    const t = (id: number, replies: number, views: number) => ({ id, replies, views });
    expect(hotOrder([t(1, 2, 9), t(2, 5, 1), t(3, 2, 30)]).map((x) => x.id)).toEqual([2, 3, 1]);
  });

  it("uses the board's palette colour, else cycles by position", () => {
    expect(boardColor({ id: 9, position: 0 })).toBe("red");
    expect(boardColor({ id: 9, position: 6 })).toBe("red");
    expect(boardColor({ id: 9, position: 1 })).not.toBe("red");
    expect(boardColor({ id: 9, position: 0, color: "#2e9d5c" })).toBe("green");
    expect(boardColor({ id: 9, position: 0, color: "#123456" })).toBe("red");
  });
});
