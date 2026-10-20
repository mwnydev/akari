"use client";

import { ApiError } from "@akari/api-client";
import { Card, ErrorState } from "@akari/ui";
import { BellRing } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  auctionCurrency,
  clockOffsetMs,
  minNextBid,
  type AuctionBid,
  type AuctionView,
} from "../../lib/auction";
import {
  auctionPanelBid,
  auctionPanelData,
  type AuctionClock,
  type AuctionRulesParams,
} from "../../lib/auction-view";
import { api, unwrap } from "../../lib/browser-api";
import { walletAuctionPayHref, walletAuctionSignHref } from "../../lib/links";
import { WALLET_ORIGIN } from "../../lib/public-config";
import { useRealtimeEvents, useRealtimeStatus, useTopic } from "../../lib/realtime/react";
import { AuctionPanel, type BidAccess } from "../forum/AuctionPanel";
import { AuctionRules } from "../forum/AuctionRules";
import { WalletLink } from "../settings/UsernameGate";

/** 实时推送不可用（轮询模式）时多久用 REST 拉一次。 */
const POLL_MS = 10_000;
/** 倒计时归零后，等服务端收尾的重试间隔（推送可能漏掉 auction_state）。 */
const END_RETRY_MS = 3_000;
const OUTBID_TOAST_MS = 8_000;

const path = (id: number) => ({ params: { path: { id } } });

/** 拍卖详情与第一页出价记录（新的在前）。 */
async function fetchAuction(id: number) {
  return Promise.all([
    unwrap(api.GET("/api/v1/auctions/{id}", path(id))),
    unwrap(api.GET("/api/v1/auctions/{id}/bids", path(id))),
  ]);
}

/** 推送来的新出价之后的下一口最低出价（与后端规则相同；币种数据不完整时保留原值，由面板报错）。 */
function nextAfter(v: AuctionView, amount: string): string {
  const currency = auctionCurrency(v.currency);
  if (!currency) return v.min_next_bid;
  return minNextBid({
    currency,
    cnyPerUsdt: v.cny_per_usdt ?? null,
    start: BigInt(v.start_price),
    increment: BigInt(v.increment),
    current: BigInt(amount),
  }).toString();
}

function endTime(iso: string): string {
  return new Date(iso).toLocaleTimeString("zh-CN", { hour12: false });
}

/**
 * 拍卖帖右侧的实时面板：首帧用服务端渲染的数据（倒计时按响应里的 server_time 算，水合一致），
 * 挂载后立即重新拉一次并按往返中点校正本机时钟；之后靠主题频道的推送更新出价、顺延与结束，断线时轮询。
 */
export function LiveAuction({
  topicId,
  initial,
  initialBids,
  rules,
  signedIn,
  login,
}: {
  topicId: number;
  initial: AuctionView;
  initialBids: AuctionBid[];
  rules: AuctionRulesParams;
  signedIn: boolean;
  /** 登录页链接（登录后回到本帖） */
  login: string;
}) {
  const id = initial.id;
  const [view, setView] = useState(initial);
  const [bids, setBids] = useState(initialBids);
  const [clock, setClock] = useState<AuctionClock>({ nowMs: 0, offsetMs: null });
  const [notice, setNotice] = useState<string>();
  const [outbid, setOutbid] = useState(false);
  const [needsWallet, setNeedsWallet] = useState(false);
  const offset = useRef<number | null>(null);
  /** 本页提交的出价金额：推送回来的同一口不算被超价（之后的出价必须更高，同一金额只能是这一口） */
  const mine = useRef(new Set<string>());

  const tick = useCallback(() => {
    if (offset.current !== null) setClock({ nowMs: Date.now(), offsetMs: offset.current });
  }, []);

  const refresh = useCallback(async () => {
    try {
      const sent = Date.now();
      const [v, page] = await fetchAuction(id);
      const received = Date.now();
      offset.current = clockOffsetMs(v.server_time, sent, received);
      setView(v);
      setBids(page.bids);
      setClock({ nowMs: received, offsetMs: offset.current });
    } catch {
      // 保留当前显示；下次推送、轮询或到点重试时再拉
    }
  }, [id]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useTopic({ topic_id: topicId });
  useRealtimeEvents((e) => {
    switch (e.type) {
      case "resync":
        return void refresh();
      case "auction_bid": {
        if (e.topic_id !== topicId || e.auction_id !== id) return;
        const own = mine.current.has(e.amount);
        setBids((bs) =>
          bs.some((b) => b.id === e.bid_id)
            ? bs
            : [
                {
                  id: e.bid_id,
                  bidder: e.bidder,
                  amount: e.amount,
                  linked: e.linked,
                  mine: own,
                  // 推送里没有出价时间：按收到的时刻（服务端时钟）显示，下次刷新换成准确时间
                  created_at: new Date(Date.now() + (offset.current ?? 0)).toISOString(),
                },
                ...bs,
              ],
        );
        if (view.ends_at && Date.parse(e.ends_at) > Date.parse(view.ends_at)) {
          setNotice(`最后时刻有新出价，结束时间顺延至 ${endTime(e.ends_at)}`);
        }
        if (!own && view.my?.leading) setOutbid(true);
        setView((v) => ({
          ...v,
          current_price: e.amount,
          min_next_bid: nextAfter(v, e.amount),
          bid_count: e.bids,
          reserve_met: e.reserve_met,
          ends_at: e.ends_at,
          ...(v.my
            ? {
                my: {
                  ...v.my,
                  leading: own,
                  outbid: own ? false : v.my.leading || v.my.outbid,
                  best_bid: own ? e.amount : (v.my.best_bid ?? null),
                },
              }
            : {}),
        }));
        return tick();
      }
      case "auction_state":
        if (e.topic_id === topicId && e.auction_id === id) void refresh();
        return;
    }
  });

  const status = useRealtimeStatus();
  useEffect(() => {
    if (status !== "polling") return;
    const timer = setInterval(() => void refresh(), POLL_MS);
    return () => clearInterval(timer);
  }, [status, refresh]);

  // 到点后服务端收尾（判定成交或流拍）：推送漏了也能自己拉到结果
  const live = view.state === "live";
  const endsAt = view.ends_at;
  useEffect(() => {
    if (!live || !endsAt || offset.current === null) return;
    const ms = Date.parse(endsAt) - (Date.now() + offset.current);
    const timer = setTimeout(() => void refresh(), Math.max(ms, 0) + END_RETRY_MS);
    return () => clearTimeout(timer);
  }, [live, endsAt, clock, refresh]);

  useEffect(() => {
    if (!outbid) return;
    const timer = setTimeout(() => setOutbid(false), OUTBID_TOAST_MS);
    return () => clearTimeout(timer);
  }, [outbid]);

  const data = auctionPanelData(view, rules, clock);
  if (!data) {
    return (
      <Card>
        <ErrorState title="拍卖数据有误" message="计价币种或汇率不完整，暂不显示价格，请稍后刷新" />
      </Card>
    );
  }

  const access: BidAccess = !signedIn
    ? {
        kind: "blocked",
        notice: (
          <a href={login} className="font-medium text-accent hover:underline">
            登录后出价
          </a>
        ),
      }
    : view.my?.seller
      ? { kind: "blocked", notice: "这是你发起的拍卖，卖家本人不能出价。" }
      : needsWallet
        ? {
            kind: "blocked",
            notice: (
              <>
                出价需要先开通交易（创建 Passkey 钱包）。{" "}
                <WalletLink
                  href={`${WALLET_ORIGIN}/setup`}
                  className="font-medium text-accent hover:underline"
                >
                  开通交易
                </WalletLink>
              </>
            ),
          }
        : {
            kind: "open",
            place: async (amount) => {
              try {
                mine.current.add(amount.toString());
                const placed = await unwrap(
                  api.POST("/api/v1/auctions/{id}/bids", {
                    ...path(id),
                    body: { amount: amount.toString() },
                  }),
                );
                setView(placed);
                tick(); // 出价可能触发顺延，倒计时按当前时刻重算
                setOutbid(false);
                // 出价记录重取一次，带上这口的「（你）」与准确时间；失败时保留推送来的那一行
                void unwrap(api.GET("/api/v1/auctions/{id}/bids", path(id))).then(
                  (page) => setBids(page.bids),
                  () => undefined,
                );
              } catch (e) {
                mine.current.delete(amount.toString());
                if (e instanceof ApiError && e.code === "WALLET_REQUIRED") setNeedsWallet(true);
                throw e;
              }
            },
          };

  const now = new Date(clock.offsetMs === null ? Date.parse(view.server_time) : clock.nowMs);
  return (
    <>
      <AuctionPanel
        auction={data}
        bids={bids.map((b) => auctionPanelBid(b, now))}
        access={access}
        payHref={walletAuctionPayHref(view.id)}
        signHref={walletAuctionSignHref(view.id)}
        notice={notice}
      />
      <AuctionRules auction={data} />
      {outbid ? (
        <div
          role="alert"
          className="fixed inset-x-4 top-20 z-50 mx-auto flex max-w-sm items-center gap-2 rounded-lg border border-border bg-surface px-4 py-3 text-sm shadow-lg"
        >
          <BellRing size={16} aria-hidden className="shrink-0 text-warning" />
          你的出价已被超过，可以再加价。
        </div>
      ) : null}
    </>
  );
}
