"use client";

import { Badge, Button, Card, Countdown, Input } from "@akari/ui";
import { SegmentedControl } from "@akari/ui/tabs";
import { BadgeCheck, Gavel, Hourglass, TimerReset } from "lucide-react";
import { useState, type FormEvent } from "react";
import { formatCnyPerUsdt } from "../../../../lib/fees";
import type { Bid } from "../_mock/data";
import { auctionUsdt, formatAuctionAmount, parseAuctionAmount, plainAuctionAmount } from "./money";

export interface AuctionTerms {
  currency: "USDT" | "CNY";
  /** 卖家签名汇率，仅 CNY */
  cnyPerUsdt?: number;
  /** 以下金额都是拍卖币种最小单位（USDT 6 位小数 / 人民币分） */
  start: string;
  increment: string;
  current: string;
  bidCount: number;
  reserveMet: boolean;
  secondsLeft: number;
  /** 成交后付款期限（小时） */
  payHours: number;
  /** 「成交待付款」预览里付款截止的剩余秒数 */
  paySecondsLeft: number;
  /** 防狙击：结束前多少分钟内有出价就顺延多少分钟，累计上限（小时） */
  snipeMinutes: number;
  snipeCapHours: number;
  /** 出价资格：钱包余额至少为出价的百分之几；0 = 不要求 */
  balancePercent: number;
}

type Phase = "live" | "sold" | "unsold";

const PHASES = [
  { value: "live", label: "进行中" },
  { value: "sold", label: "成交待付款" },
  { value: "unsold", label: "流拍" },
] as const satisfies readonly { value: Phase; label: string }[];

const QUICK_STEPS = [1, 2, 5] as const;

/**
 * 拍卖面板：当前价（人民币拍卖同时显示按卖家汇率折合的 USDT）、条款、倒计时、出价框与出价记录。
 * 顶部的状态切换只为设计稿预览三种状态。出价不上链、不花 gas，先确认再提交。
 */
export function AuctionPanel({
  terms: t,
  bids: initialBids,
  viewer,
}: {
  terms: AuctionTerms;
  bids: Bid[];
  /** 已脱敏的访问者名字，用于自己的出价记录 */
  viewer: string;
}) {
  const [phase, setPhase] = useState<Phase>("live");
  const [bids, setBids] = useState(initialBids);
  const [draft, setDraft] = useState("");
  const [pending, setPending] = useState<bigint | null>(null);
  const [status, setStatus] = useState("");

  const cur = t.currency;
  const fmt = (v: bigint) => formatAuctionAmount(cur, v);
  const usdtOf = (v: bigint) =>
    cur === "CNY" && t.cnyPerUsdt
      ? formatAuctionAmount("USDT", auctionUsdt(v, t.cnyPerUsdt))
      : null;

  const current = BigInt(bids[0]?.amount ?? t.current);
  const increment = BigInt(t.increment);
  const minBid = bids.length > 0 ? current + increment : BigInt(t.start);
  const parsed = draft ? parseAuctionAmount(cur, draft) : null;
  const error =
    draft === ""
      ? undefined
      : parsed === null
        ? "金额格式不对"
        : parsed < minBid
          ? `至少出 ${fmt(minBid)}`
          : undefined;
  const bidCount = t.bidCount + (bids.length - initialBids.length);

  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (parsed === null || error) return;
    setPending(parsed);
  };
  const confirm = () => {
    if (pending === null) return;
    setBids((bs) => [
      {
        id: (bs[0]?.id ?? 0) + 1,
        bidder: `${viewer}（你）`,
        amount: pending.toString(),
        time: "刚刚",
        timeIso: new Date().toISOString(),
      },
      ...bs,
    ]);
    setPending(null);
    setDraft("");
    setStatus("出价成功，你是当前最高出价者。被超价时会通知你（站内 + Telegram）。");
  };

  const sold = phase === "sold";
  // 流拍必然未达保留价；进行中按实际情况显示
  const reserveMet = phase === "unsold" ? false : t.reserveMet;
  const usdt = usdtOf(current);

  return (
    <div className="space-y-3">
      <Card className="space-y-4">
        <div className="flex flex-wrap items-center justify-between gap-2">
          {phase === "live" ? (
            <Badge tone="accent">
              <span className="size-1.5 animate-pulse-soft rounded-full bg-accent" aria-hidden />
              拍卖进行中
            </Badge>
          ) : sold ? (
            <Badge tone="accent">成交待付款</Badge>
          ) : (
            <Badge>流拍</Badge>
          )}
          <SegmentedControl
            label="预览拍卖状态（设计稿）"
            value={phase}
            onValueChange={(p) => {
              setPhase(p);
              setPending(null);
            }}
            options={[...PHASES]}
            className="text-xs"
          />
        </div>

        <div>
          <p className="text-sm text-fg-muted">
            {phase === "live" ? "当前价" : sold ? "成交价" : "最高出价"}
          </p>
          <p className="amount text-4xl font-semibold tracking-tight">{fmt(current)}</p>
          {usdt && t.cnyPerUsdt ? (
            <p className="amount mt-0.5 text-sm text-fg-muted">
              按卖家汇率 {formatCnyPerUsdt(t.cnyPerUsdt)} 折合 {usdt}
            </p>
          ) : null}
        </div>

        <dl className="grid grid-cols-2 gap-2 text-sm">
          {[
            ["起拍价", fmt(BigInt(t.start))],
            ["加价幅度", fmt(increment)],
            ["出价次数", `${bidCount} 次`],
          ].map(([k, v]) => (
            <div key={k} className="rounded-md bg-surface-2 px-3 py-2">
              <dt className="text-xs text-fg-muted">{k}</dt>
              <dd className="amount font-medium">{v}</dd>
            </div>
          ))}
          <div className="rounded-md bg-surface-2 px-3 py-2">
            <dt className="text-xs text-fg-muted">保留价</dt>
            <dd>
              <Badge tone={reserveMet ? "success" : "neutral"}>
                {reserveMet ? "已达保留价" : "未达保留价"}
              </Badge>
            </dd>
          </div>
        </dl>

        {phase === "live" ? (
          <>
            <div className="rounded-lg border border-border p-3">
              <p className="flex items-center justify-between gap-2">
                <span className="flex items-center gap-1.5 text-sm text-fg-muted">
                  <Hourglass size={15} aria-hidden />
                  距结束
                </span>
                <Countdown seconds={t.secondsLeft} className="text-xl font-semibold" />
              </p>
              <p className="mt-1 flex gap-1.5 text-xs text-fg-muted">
                <TimerReset size={14} aria-hidden className="mt-0.5 shrink-0" />
                结束前 {t.snipeMinutes} 分钟内有新出价，自动顺延 {t.snipeMinutes} 分钟（累计最多{" "}
                {t.snipeCapHours} 小时）
              </p>
            </div>

            {pending === null ? (
              <form onSubmit={submit} className="space-y-2" noValidate>
                <label className="block text-sm font-medium" htmlFor="bid-amount">
                  你的出价（{cur === "CNY" ? "人民币" : "USDT"}）
                </label>
                <div className="flex gap-2">
                  <Input
                    id="bid-amount"
                    inputMode="decimal"
                    autoComplete="off"
                    placeholder={plainAuctionAmount(cur, minBid)}
                    value={draft}
                    onChange={(e) => setDraft(e.target.value)}
                    aria-invalid={error ? true : undefined}
                    aria-describedby="bid-hint"
                    className="amount"
                  />
                  <Button type="submit" disabled={parsed === null || error !== undefined}>
                    <Gavel size={16} aria-hidden />
                    出价
                  </Button>
                </div>
                <p
                  id="bid-hint"
                  className={error ? "text-sm text-danger" : "text-sm text-fg-muted"}
                  role={error ? "alert" : undefined}
                >
                  {error ?? `最低出价 ${fmt(minBid)}（当前价 + 加价幅度）`}
                </p>
                <div className="flex gap-2" role="group" aria-label="快捷加价">
                  {QUICK_STEPS.map((k) => {
                    const v = current + increment * BigInt(k);
                    return (
                      <Button
                        key={k}
                        variant="secondary"
                        size="sm"
                        className="amount flex-1"
                        aria-label={`出价 ${fmt(v)}`}
                        onClick={() => setDraft(plainAuctionAmount(cur, v))}
                      >
                        +{fmt(increment * BigInt(k))}
                      </Button>
                    );
                  })}
                </div>
              </form>
            ) : (
              <div
                className="space-y-2 rounded-lg bg-accent-soft p-3"
                role="group"
                aria-label="确认出价"
              >
                <p className="font-medium">确认出价 {fmt(pending)}？</p>
                {usdtOf(pending) ? (
                  <p className="amount text-sm">
                    成交时应付 {usdtOf(pending)}（按卖家汇率换算，向上取整）
                  </p>
                ) : null}
                <p className="text-xs text-fg-muted">
                  出价不能撤回；成交后需在 {t.payHours} 小时内付款，逾期记违约并禁拍。
                </p>
                <div className="flex gap-2">
                  <Button className="flex-1" onClick={confirm} autoFocus>
                    确认出价
                  </Button>
                  <Button variant="secondary" onClick={() => setPending(null)}>
                    返回修改
                  </Button>
                </div>
              </div>
            )}
            <p role="status" className="text-sm text-success empty:hidden">
              {status}
            </p>
          </>
        ) : sold ? (
          <div className="space-y-2 rounded-lg bg-accent-soft p-3 text-sm">
            <p className="flex items-center gap-1.5 font-medium">
              <BadgeCheck size={16} aria-hidden className="text-accent" />
              平台已签发成交证明 · 赢家 {bids[0]?.bidder}
            </p>
            <p className="flex items-center justify-between gap-2">
              <span className="text-fg-muted">付款截止还剩</span>
              <Countdown seconds={t.paySecondsLeft} className="font-semibold" ended="已逾期" />
            </p>
            <p className="text-xs text-fg-muted">
              赢家付款后资金进入担保合约，之后与普通担保交易相同；逾期未付记违约、禁拍 30
              天，由次高出价者递补。
            </p>
            <Button className="w-full">赢家去付款</Button>
          </div>
        ) : (
          <div className="rounded-lg bg-surface-2 p-3 text-sm">
            <p className="font-medium">未达保留价，本场流拍</p>
            <p className="mt-1 text-xs text-fg-muted">
              不建订单、不扣任何费用；卖家可以调整条件后重新挂拍。
            </p>
          </div>
        )}
      </Card>

      <div className="rounded-md border border-border">
        <h2 className="flex items-center justify-between border-b border-border px-4 py-2 text-sm font-medium">
          出价记录
          <span className="text-xs font-normal text-fg-muted">只追加、公开可查 · 出价人已脱敏</span>
        </h2>
        <ol className="divide-y divide-border text-sm" aria-label="出价记录">
          {bids.map((b, i) => (
            <li key={b.id} className="flex items-center gap-3 px-4 py-2">
              <span className="w-20 truncate">{b.bidder}</span>
              <span className="amount flex-1 font-medium">{fmt(BigInt(b.amount))}</span>
              {i === 0 ? <Badge tone="accent">最高</Badge> : null}
              <time dateTime={b.timeIso} className="amount text-xs text-fg-muted">
                {b.time}
              </time>
            </li>
          ))}
        </ol>
      </div>
    </div>
  );
}
