import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { AuctionState } from "../../lib/auction";
import {
  AuctionPanel,
  type AuctionMyStatus,
  type AuctionPanelData,
  type BidAccess,
} from "./AuctionPanel";

const base: AuctionPanelData = {
  state: "live",
  currency: "CNY",
  cnyPerUsdt: 72_000,
  start: "50000",
  increment: "2000",
  current: "128000",
  minNext: "130000",
  bidCount: 23,
  reserveMet: true,
  secondsLeft: 2 * 3600 + 14 * 60 + 5,
  endsAt: "2026-10-20T10:14:05Z",
  extendedSecs: 0,
  snipeWindowSecs: 900,
  snipeExtendSecs: 900,
  maxExtensionSecs: 86_400,
  payWindowSecs: 172_800,
  runnerUpWindowSecs: 86_400,
  defaultBanSecs: 30 * 86_400,
  paySecondsLeft: 47 * 3600,
  my: "none",
  eligibility: { kind: "none" },
  feePayer: 0,
};

const open: BidAccess = { kind: "open", place: async () => {} };

function render(
  over: Partial<AuctionPanelData> & { state?: AuctionState; my?: AuctionMyStatus },
  access: BidAccess = open,
) {
  return renderToStaticMarkup(
    createElement(AuctionPanel, {
      auction: { ...base, ...over },
      bids: [],
      access,
      payHref: "https://wallet.test/auction/pay?auction=7",
      signHref: "https://wallet.test/auction/sign?auction=7",
    }),
  );
}

describe("auction panel states", () => {
  it("live: price with seller-rate USDT, countdown, anti-snipe rule, bid box with next minimum", () => {
    const html = render({});
    expect(html).toContain("拍卖进行中");
    expect(html).toContain("¥1,280.00");
    expect(html).toContain("按卖家汇率 7.2000 折合 177.777778 USDT");
    expect(html).toContain("2:14:05");
    expect(html).toContain("结束前 15 分钟内有新出价，自动顺延 15 分钟（累计最多 24 小时）");
    // 最低出价 ¥1,300.00 折合 180.555556 USDT（向上取整）
    expect(html).toContain("最低出价 ¥1,300.00（折合 180.555556 USDT）");
    expect(html).toContain('id="bid-amount"');
    expect(html).not.toContain("去付款");
  });

  it("live without bids: start price is the minimum; guests see a login link instead of the box", () => {
    const html = render(
      {
        current: null,
        bidCount: 0,
        currency: "USDT",
        cnyPerUsdt: null,
        start: "20000000",
        increment: "1000000",
        minNext: "20000000",
      },
      { kind: "blocked", notice: "登录后出价" },
    );
    expect(html).toContain("起拍价");
    expect(html).toContain("20.00 USDT");
    expect(html).not.toContain("折合");
    expect(html).not.toContain('id="bid-amount"');
    expect(html).toContain("登录后出价");
  });

  it("anti-snipe window: the countdown turns to the warning tone and shows the extension", () => {
    const html = render({ secondsLeft: 600, extendedSecs: 1800 });
    expect(html).toContain("text-warning");
    expect(html).toContain("已顺延 30 分钟");
  });

  it("pending payment: the winner gets 去付款, everyone else sees the deadline", () => {
    const winner = render({ state: "awaiting_payment", my: "winner" });
    expect(winner).toContain("成交待付款");
    expect(winner).toContain("你赢得了本场拍卖");
    expect(winner).toContain("https://wallet.test/auction/pay?auction=7");
    expect(winner).toContain("1 天 23 小时");
    expect(winner).toContain("禁拍 30 天");
    const other = render({ state: "awaiting_payment", my: "outbid" });
    expect(other).toContain("等待赢家付款");
    expect(other).toContain("付款截止还剩");
    expect(other).not.toContain("去付款");
  });

  it("runner-up offered: the runner-up can pay at their own bid", () => {
    const html = render({ state: "awaiting_payment", my: "runner_up_offered" });
    expect(html).toContain("原赢家逾期未付款，你可以按自己的出价递补成交");
    expect(html).toContain("去付款");
  });

  it("ended: settling until the worker issues the proof or marks it unsold", () => {
    const html = render({ state: "ended", secondsLeft: 0 });
    expect(html).toContain("已结束 · 结算中");
    expect(html).toContain("拍卖已结束，正在结算");
    expect(html).not.toContain('id="bid-amount"');
  });

  it("linked bidders are flagged in the history", () => {
    const html = renderToStaticMarkup(
      createElement(AuctionPanel, {
        auction: base,
        bids: [
          {
            id: 2,
            bidder: "夜***风",
            amount: "130000",
            time: "10:00",
            timeIso: "2026-10-20T10:00:00Z",
            mine: false,
            linked: true,
          },
        ],
        access: open,
      }),
    );
    expect(html).toContain("关联账号");
  });

  it("unsold, settled, draft", () => {
    const unsold = render({ state: "unsold", reserveMet: true });
    expect(unsold).toContain("未达保留价，本场流拍");
    expect(unsold).toContain("未达保留价</span>");
    expect(render({ state: "unsold", bidCount: 0, current: null })).toContain("无人出价，本场流拍");
    expect(render({ state: "settled" })).toContain("担保订单已建立");
    expect(render({ state: "draft", my: "seller" })).toContain("去钱包签名");
    expect(render({ state: "draft", my: "none" })).not.toContain("去钱包签名");
  });
});
