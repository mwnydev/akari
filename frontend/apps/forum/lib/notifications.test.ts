import { describe, expect, it } from "vitest";
import { describeNotification, type Notification } from "./notifications";

const n = (kind: string, payload: Record<string, unknown>): Notification => ({
  id: 1,
  kind,
  actor_id: 5,
  payload,
  read: false,
  created_at: "2026-10-19T00:00:00Z",
});

describe("describeNotification", () => {
  it("links replies and mentions to the right page and floor", () => {
    expect(
      describeNotification(n("reply", { topic_id: 9, post_id: 3, floor: 25 }), "小林"),
    ).toEqual({ text: "小林 回复了你（#25 楼）", href: "/t/9?page=2#floor-25" });
    expect(describeNotification(n("mention", { topic_id: 9, floor: 2 }), "小林")).toEqual({
      text: "小林 在 #2 楼提到了你",
      href: "/t/9#floor-2",
    });
  });

  it("links messages and offers to the conversation", () => {
    expect(describeNotification(n("message", { conversation_id: 4 }), "a").href).toBe(
      "/messages/4",
    );
    expect(
      describeNotification(n("offer", { conversation_id: 4, offer_id: 2, event: "created" }), "a"),
    ).toEqual({ text: "a 发来一份担保报价", href: "/messages/4" });
  });

  it("describes order updates including reorg corrections", () => {
    expect(
      describeNotification(
        n("order", { order_id: 7, state: "funded", level: "latest", correction: true }),
        null,
      ),
    ).toEqual({ text: "订单 #7 已付款（区块回滚后的状态修正）", href: "/orders/7" });
  });

  it("tells reporters only the outcome of their report", () => {
    expect(
      describeNotification(
        n("system", { event: "report_resolved", outcome: "actioned", topic_id: 9, floor: 25 }),
        null,
      ),
    ).toEqual({ text: "你举报的内容（#25 楼）已处理，感谢反馈", href: "/t/9?page=2#floor-25" });
    expect(
      describeNotification(
        n("system", { event: "report_resolved", outcome: "dismissed", topic_id: 9, floor: 1 }),
        null,
      ).text,
    ).toBe("你举报的内容（#1 楼）经核查未发现违规");
  });

  it("describes auction events for the recipient only", () => {
    // 2026-10-19 09:04 北京时间
    const at = Date.UTC(2026, 9, 19, 1, 4) / 1000;
    expect(
      describeNotification(
        n("auction", { auction_id: 7, topic_id: 9, event: "won", pay_deadline: at }),
        null,
      ),
    ).toEqual({ text: "你已中标拍卖 #7，请在 2026-10-19 09:04 前付款", href: "/t/9" });
    expect(
      describeNotification(
        n("auction", { auction_id: 7, topic_id: 9, event: "pay_reminder", pay_deadline: at }),
        null,
      ).text,
    ).toBe("拍卖 #7 付款截止时间快到了，请在 2026-10-19 09:04 前付款");
    expect(
      describeNotification(
        n("auction", { auction_id: 7, topic_id: 9, event: "ending_soon", ends_at: at }),
        null,
      ).text,
    ).toBe("你出过价的拍卖 #7 即将结束（2026-10-19 09:04）");
    expect(describeNotification(n("auction", { auction_id: 7, event: "runner_up" }), null)).toEqual(
      { text: "拍卖 #7 的中标者未付款，轮到你按出价成交，请尽快付款", href: null },
    );
    expect(describeNotification(n("auction", { auction_id: 7, event: "x" }), null).text).toBe(
      "拍卖 #7 有新动态",
    );
  });

  it("falls back gracefully on unexpected payloads", () => {
    expect(describeNotification(n("reply", { topic_id: "x" }), null)).toEqual({
      text: "有人 回复了你",
      href: null,
    });
    expect(describeNotification(n("offer", { event: "brand_new" }), "a").text).toBe(
      "a 更新了担保报价",
    );
    expect(describeNotification(n("system", { event: "x" }), null)).toEqual({
      text: "系统通知",
      href: null,
    });
  });
});
