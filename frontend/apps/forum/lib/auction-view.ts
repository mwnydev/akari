// 拍卖接口数据 → 拍卖面板的显示模型（纯函数，单元测试直接测）。
import type {
  AuctionMyStatus,
  AuctionPanelBid,
  AuctionPanelData,
} from "../components/forum/AuctionPanel";
import {
  auctionCurrency,
  secondsAtResponse,
  secondsUntil,
  type AuctionBid,
  type AuctionSettings,
  type AuctionView,
} from "./auction";
import { formatShortTime } from "./display";

/** 剩余秒数怎么算：`offsetMs` 为 null 时按响应里的 server_time（服务端渲染与水合首帧），否则按校正后的本机时钟。 */
export interface AuctionClock {
  nowMs: number;
  offsetMs: number | null;
}

/** 站长设置里面板用到的几项（`GET /auctions/params` 的 `settings`）。 */
export interface AuctionRulesParams {
  payWindowSecs: number;
  runnerUpWindowSecs: number;
  defaultBanSecs: number;
}

export function auctionRulesParams(
  s: Pick<AuctionSettings, "pay_window_secs" | "runner_up_window_secs" | "default_ban_secs">,
): AuctionRulesParams {
  return {
    payWindowSecs: s.pay_window_secs,
    runnerUpWindowSecs: s.runner_up_window_secs,
    defaultBanSecs: s.default_ban_secs,
  };
}

function remaining(iso: string, view: AuctionView, clock: AuctionClock): number {
  return clock.offsetMs === null
    ? secondsAtResponse(iso, view.server_time)
    : secondsUntil(iso, clock.nowMs, clock.offsetMs);
}

/** 后端的 `my`（登录用户才有）→ 面板上的一种关系；成交证明给了我时按轮次区分赢家与递补。 */
export function myStatus(view: AuctionView): AuctionMyStatus {
  const my = view.my;
  if (!my) return "none";
  if (my.seller) return "seller";
  if (my.winner) return view.round > 1 ? "runner_up_offered" : "winner";
  if (my.leading) return "leading";
  return my.outbid ? "outbid" : "none";
}

/** 已顺延的秒数：结束时间 − 开始时间 − 拍卖时长（草稿没有开始时间时为 0）。 */
function extendedSecs(view: AuctionView): number {
  if (!view.starts_at || !view.ends_at) return 0;
  const secs = (Date.parse(view.ends_at) - Date.parse(view.starts_at)) / 1000 - view.duration_secs;
  return Math.max(0, Math.round(secs));
}

/** 币种不认识时返回 null：调用方显示出错，不按猜测的币种显示金额。 */
export function auctionPanelData(
  view: AuctionView,
  rules: AuctionRulesParams,
  clock: AuctionClock,
): AuctionPanelData | null {
  const currency = auctionCurrency(view.currency);
  const rate = view.cny_per_usdt ?? null;
  if (!currency || (currency === "CNY") !== (rate !== null)) return null;
  return {
    state: view.state,
    currency,
    cnyPerUsdt: rate,
    start: view.start_price,
    increment: view.increment,
    // 后端在还没有出价时给的是起拍价
    current: view.bid_count > 0 ? view.current_price : null,
    minNext: view.min_next_bid,
    bidCount: view.bid_count,
    reserveMet: view.reserve_met,
    secondsLeft: view.ends_at ? remaining(view.ends_at, view, clock) : 0,
    endsAt: view.ends_at ?? "",
    extendedSecs: extendedSecs(view),
    snipeWindowSecs: view.snipe_window_secs ?? 0,
    snipeExtendSecs: view.snipe_extension_secs ?? 0,
    maxExtensionSecs: view.extension_cap_secs ?? 0,
    ...rules,
    paySecondsLeft: view.pay_deadline ? remaining(view.pay_deadline, view, clock) : null,
    my: myStatus(view),
    eligibility: view.eligibility,
    feePayer: view.terms.fee_payer ?? 0,
  };
}

export function auctionPanelBid(b: AuctionBid, now: Date): AuctionPanelBid {
  return {
    id: b.id,
    bidder: b.bidder,
    amount: b.amount,
    time: formatShortTime(b.created_at, now),
    timeIso: b.created_at,
    mine: b.mine,
    linked: b.linked,
  };
}
