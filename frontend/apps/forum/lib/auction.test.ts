import { cnyFenToUsdt } from "@akari/fees";
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import {
  auctionCurrency,
  auctionUsdt,
  clockOffsetMs,
  formatAuctionAmount,
  minIncrement,
  minNextBid,
  parseAuctionAmount,
  plainAuctionAmount,
  secondsAtResponse,
  secondsUntil,
} from "./auction";
import type { AuctionView } from "./auction";
import { auctionPanelData, auctionRulesParams, myStatus } from "./auction-view";

interface CnyVector {
  name: string;
  priceFen: string;
  cnyPerUsdt: number;
  expect: string;
}

const vectors = JSON.parse(
  readFileSync(new URL("../../../../test-vectors/fees.json", import.meta.url), "utf8"),
) as { cny: { toUsdt: CnyVector[] } };

describe("server clock", () => {
  const server = "2026-10-20T08:00:00.000Z";
  const serverMs = Date.parse(server);

  it("takes the offset at the midpoint of the round trip", () => {
    // 本机慢 90 秒，往返 400 毫秒：服务端时刻对应本机的 sent + 200
    const sent = serverMs - 90_000 - 200;
    expect(clockOffsetMs(server, sent, sent + 400)).toBe(90_000);
    expect(clockOffsetMs(server, serverMs + 5_000, serverMs + 5_000)).toBe(-5_000);
  });

  it("counts down on the server clock, not the local one", () => {
    const ends = "2026-10-20T08:10:00.000Z";
    // 本机快 2 分钟：本机显示 08:02 时服务端才 08:00，还剩 10 分钟
    const fastLocal = serverMs + 120_000;
    expect(secondsUntil(ends, fastLocal, -120_000)).toBe(600);
    expect(secondsUntil(ends, fastLocal + 1_500, -120_000)).toBe(598);
    // 不校正会少显示 2 分钟
    expect(secondsUntil(ends, fastLocal, 0)).toBe(480);
    expect(secondsUntil(ends, serverMs + 700_000, 0)).toBe(0);
  });

  it("first frame uses the response's server_time only (same on server and client)", () => {
    expect(secondsAtResponse("2026-10-20T10:14:05Z", "2026-10-20T08:00:00Z")).toBe(8045);
    expect(secondsAtResponse("2026-10-20T07:00:00Z", "2026-10-20T08:00:00Z")).toBe(0);
  });
});

describe("bid minimums", () => {
  it("first bid is the start price, then current + max(increment, 1 USDT, 1% of current)", () => {
    const cny = { currency: "CNY" as const, cnyPerUsdt: 72_000, start: 50_000n, increment: 2_000n };
    expect(minNextBid({ ...cny, current: null })).toBe(50_000n);
    expect(minNextBid({ ...cny, current: 128_000n })).toBe(130_000n);
    // ¥3,000 的 1% = ¥30 大于幅度 ¥20（与后端 min_step 的用例一致）
    expect(minNextBid({ ...cny, current: 300_000n })).toBe(303_000n);
    // 幅度低于 1 USDT 时按 1 USDT
    const usdt = { currency: "USDT" as const, cnyPerUsdt: null, start: 10_000_000n, increment: 1n };
    expect(minNextBid({ ...usdt, current: 10_000_000n })).toBe(11_000_000n);
  });

  it("increment floor is max(1 USDT, 1% of the start price)", () => {
    expect(minIncrement("USDT", 20_000_000n, null)).toBe(1_000_000n);
    expect(minIncrement("USDT", 250_000_000n, null)).toBe(2_500_000n);
    // 1% 向上取整到最小单位
    expect(minIncrement("USDT", 150_000_001n, null)).toBe(1_500_001n);
  });

  it("CNY: 1 USDT is rounded up to the fen, so the floor never converts below 1 USDT", () => {
    expect(minIncrement("CNY", 50_000n, 72_000)).toBe(720n); // ¥7.20
    expect(minIncrement("CNY", 50_000n, 72_001)).toBe(721n);
    expect(cnyFenToUsdt(721n, 72_001n)).toBeGreaterThanOrEqual(1_000_000n);
    expect(cnyFenToUsdt(720n, 72_001n)).toBeLessThan(1_000_000n);
    // 起拍 ¥1,000：1% = ¥10.00 > ¥7.20
    expect(minIncrement("CNY", 100_000n, 72_000)).toBe(1_000n);
  });
});

describe("amounts", () => {
  it("formats, prints for inputs and parses both currencies", () => {
    expect(formatAuctionAmount("CNY", 128_000n)).toBe("¥1,280.00");
    expect(formatAuctionAmount("USDT", 36_000_000n)).toBe("36.00 USDT");
    expect(plainAuctionAmount("CNY", 130_005n)).toBe("1300.05");
    expect(plainAuctionAmount("USDT", 1_234_500_000n)).toBe("1234.50");
    expect(parseAuctionAmount("CNY", "1300.5")).toBe(130_050n);
    expect(parseAuctionAmount("CNY", "1300.555")).toBeNull();
    expect(parseAuctionAmount("USDT", "12.345678")).toBe(12_345_678n);
    expect(auctionCurrency("cny")).toBe("CNY");
    expect(auctionCurrency("eur")).toBeNull();
  });

  it.each(vectors.cny.toUsdt.map((v) => [v.name, v] as const))(
    "CNY → USDT shown to bidders matches the contract vector: %s",
    (_name, v) => {
      const usdt = auctionUsdt("CNY", BigInt(v.priceFen), v.cnyPerUsdt);
      expect(usdt).toBe(BigInt(v.expect));
      // 显示不丢位：页面上的数字解析回来就是合约金额
      const [whole = "", frac = ""] = formatAuctionAmount("USDT", usdt)
        .replace(" USDT", "")
        .replaceAll(",", "")
        .split(".");
      expect(BigInt(whole) * 1_000_000n + BigInt(frac.padEnd(6, "0"))).toBe(usdt);
    },
  );

  it("USDT auctions settle at the price itself", () => {
    expect(auctionUsdt("USDT", 88_500_000n, null)).toBe(88_500_000n);
  });
});

const view: AuctionView = {
  id: 7,
  topic_id: 103,
  forum_id: 1,
  seller_id: 9,
  state: "live",
  currency: "cny",
  cny_per_usdt: 72_000,
  start_price: "50000",
  increment: "2000",
  reserve_met: true,
  current_price: "128000",
  current_usdt: "17777778",
  min_next_bid: "130000",
  bid_count: 23,
  eligibility: { kind: "none" },
  terms: { instant: false, ship_window: 259_200, confirm_window: 604_800, fee_payer: 0 },
  starts_at: "2026-10-17T10:00:00Z",
  ends_at: "2026-10-20T10:14:05Z",
  duration_secs: 3 * 86_400,
  snipe_window_secs: 900,
  snipe_extension_secs: 900,
  extension_cap_secs: 86_400,
  server_time: "2026-10-20T08:00:00Z",
  round: 0,
};
const rules = auctionRulesParams({
  pay_window_secs: 48 * 3600,
  runner_up_window_secs: 24 * 3600,
  default_ban_secs: 30 * 86_400,
});

describe("auction view → panel", () => {
  it("uses server_time before the clock is measured, the corrected clock after", () => {
    expect(auctionPanelData(view, rules, { nowMs: 0, offsetMs: null })?.secondsLeft).toBe(8045);
    const local = Date.parse("2026-10-20T08:00:00Z") + 60_000; // 本机快 1 分钟
    expect(auctionPanelData(view, rules, { nowMs: local, offsetMs: -60_000 })?.secondsLeft).toBe(
      8045,
    );
  });

  it("derives the extension from start, end and duration; no bids means no current price", () => {
    const clock = { nowMs: 0, offsetMs: null };
    const data = auctionPanelData(view, rules, clock);
    expect(data?.extendedSecs).toBe(14 * 60 + 5);
    expect(data?.current).toBe("128000");
    expect(data?.payWindowSecs).toBe(48 * 3600);
    expect(auctionPanelData({ ...view, bid_count: 0 }, rules, clock)?.current).toBeNull();
    // 草稿：还没有开始与结束时间
    const draft = auctionPanelData(
      { ...view, state: "draft", starts_at: null, ends_at: null },
      rules,
      clock,
    );
    expect([draft?.secondsLeft, draft?.extendedSecs]).toEqual([0, 0]);
  });

  it("maps my status, telling the winner from a runner-up by the proof round", () => {
    const my = { seller: false, leading: false, outbid: false, winner: false };
    expect(myStatus(view)).toBe("none");
    expect(myStatus({ ...view, my: { ...my, seller: true } })).toBe("seller");
    expect(myStatus({ ...view, my: { ...my, leading: true } })).toBe("leading");
    expect(myStatus({ ...view, my: { ...my, outbid: true } })).toBe("outbid");
    expect(myStatus({ ...view, round: 1, my: { ...my, winner: true } })).toBe("winner");
    expect(myStatus({ ...view, round: 2, my: { ...my, winner: true } })).toBe("runner_up_offered");
  });

  it("refuses inconsistent currency data instead of guessing", () => {
    const clock = { nowMs: 0, offsetMs: null };
    expect(auctionPanelData({ ...view, cny_per_usdt: null }, rules, clock)).toBeNull();
    expect(auctionPanelData({ ...view, currency: "usdt" }, rules, clock)).toBeNull();
    expect(
      auctionPanelData({ ...view, currency: "usdt", cny_per_usdt: null }, rules, clock),
    ).not.toBeNull();
  });
});
