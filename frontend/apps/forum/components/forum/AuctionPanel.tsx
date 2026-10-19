"use client";

import { Badge, Button, Card, Countdown, Input, cn } from "@akari/ui";
import { BadgeCheck, Gavel, Hourglass, TimerReset } from "lucide-react";
import { useState, type FormEvent, type ReactNode } from "react";
import {
  auctionUsdt,
  days,
  formatAuctionAmount,
  hours,
  parseAuctionAmount,
  plainAuctionAmount,
  type AuctionCurrency,
  type AuctionEligibility,
  type AuctionState,
} from "../../lib/auction";
import { formatCnyPerUsdt } from "../../lib/fees";

/** 访问者与这场拍卖的关系（游客为 none）；runner_up_offered：原赢家逾期，递补的成交证明给了我。 */
export type AuctionMyStatus =
  "none" | "seller" | "leading" | "outbid" | "winner" | "runner_up_offered";

/** 拍卖面板的显示模型（正式页面由接口数据换算，设计稿由模拟数据换算）。金额是拍卖币种最小单位的十进制字符串。 */
export interface AuctionPanelData {
  state: AuctionState;
  currency: AuctionCurrency;
  /** 卖家签名汇率（4 位小数定点数），仅人民币拍卖 */
  cnyPerUsdt: number | null;
  start: string;
  increment: string;
  /** 还没有人出价时为 null */
  current: string | null;
  /** 下一口最低出价 */
  minNext: string;
  bidCount: number;
  reserveMet: boolean;
  /** 按服务端时钟校正后的剩余秒数 */
  secondsLeft: number;
  /** 结束时间（顺延后会变，倒计时据此重新开始） */
  endsAt: string;
  extendedSecs: number;
  snipeWindowSecs: number;
  snipeExtendSecs: number;
  maxExtensionSecs: number;
  payWindowSecs: number;
  /** 递补者的付款期 */
  runnerUpWindowSecs: number;
  /** 违约自动禁拍时长 */
  defaultBanSecs: number;
  /** 成交待付款：付款截止的剩余秒数 */
  paySecondsLeft: number | null;
  my: AuctionMyStatus;
  eligibility: AuctionEligibility;
  /** 0 卖家、1 买家、2 各半 */
  feePayer: number;
}

export interface AuctionPanelBid {
  id: number;
  bidder: string;
  amount: string;
  time: string;
  timeIso: string;
  mine: boolean;
  /** 与本场其他出价人是关联账号（同设备） */
  linked: boolean;
}

/** 出价入口：能出价时给提交函数（失败时抛出带中文 message 的错误），不能时给说明（登录、开通交易、卖家本人）。 */
export type BidAccess =
  | { kind: "open"; place: (amount: bigint) => Promise<void> }
  | { kind: "blocked"; notice: ReactNode };

const QUICK_STEPS = [1, 2, 5] as const;
const MINUTE = 60;

const STATE_BADGE: Record<AuctionState, { label: string; tone: "accent" | "success" | "neutral" }> =
  {
    draft: { label: "待卖家签名", tone: "neutral" },
    live: { label: "拍卖进行中", tone: "accent" },
    ended: { label: "已结束 · 结算中", tone: "neutral" },
    awaiting_payment: { label: "成交待付款", tone: "accent" },
    settled: { label: "已成交", tone: "success" },
    unsold: { label: "流拍", tone: "neutral" },
    cancelled: { label: "已取消", tone: "neutral" },
  };

/** 成交后买家在货款之外还要付的部分；具体金额只在钱包付款页按链上参数算出，这里不写数字。 */
const EXTRA_COST = ["付款时另付代付费", "付款时另付手续费与代付费", "付款时另付一半手续费与代付费"];

function priceLabel(a: AuctionPanelData): string {
  if (a.state === "awaiting_payment" || a.state === "settled") return "成交价";
  if (a.current === null) return "起拍价";
  return a.state === "live" ? "当前价" : "最高出价";
}

export function AuctionPanel({
  auction: a,
  bids,
  access,
  payHref,
  signHref,
  notice,
  header,
}: {
  auction: AuctionPanelData;
  bids: AuctionPanelBid[];
  access: BidAccess;
  /** 赢家（或递补者）去钱包付款 */
  payHref?: string | undefined;
  /** 卖家去钱包签拍卖授权（草稿状态） */
  signHref?: string | undefined;
  /** 实时提示（如防狙击顺延），读屏软件会播报 */
  notice?: string | undefined;
  /** 标题行右侧（设计稿放状态切换） */
  header?: ReactNode;
}) {
  const cur = a.currency;
  const fmt = (v: bigint) => formatAuctionAmount(cur, v);
  /** 人民币拍卖价按卖家汇率折合的 USDT，与合约换算一致 */
  const usdtOf = (v: bigint) =>
    cur === "CNY" ? formatAuctionAmount("USDT", auctionUsdt(cur, v, a.cnyPerUsdt)) : null;
  const shown = BigInt(a.current ?? a.start);
  const usdt = usdtOf(shown);
  const badge = STATE_BADGE[a.state];
  const live = a.state === "live";
  const reserveMet = a.state === "unsold" ? false : a.reserveMet;

  return (
    <div className="space-y-3">
      <Card className="space-y-4">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <Badge tone={badge.tone}>
            {live ? (
              <span className="size-1.5 animate-pulse-soft rounded-full bg-accent" aria-hidden />
            ) : null}
            {badge.label}
          </Badge>
          {header}
        </div>

        <div>
          <p className="text-sm text-fg-muted">{priceLabel(a)}</p>
          <p className="amount text-4xl font-semibold tracking-tight" data-testid="auction-price">
            {fmt(shown)}
          </p>
          {usdt && a.cnyPerUsdt !== null ? (
            <p className="amount mt-0.5 text-sm text-fg-muted">
              按卖家汇率 {formatCnyPerUsdt(a.cnyPerUsdt)} 折合 {usdt}
            </p>
          ) : null}
        </div>

        <dl className="grid grid-cols-2 gap-2 text-sm">
          {[
            ["起拍价", fmt(BigInt(a.start))],
            ["加价幅度", fmt(BigInt(a.increment))],
            ["出价次数", `${a.bidCount} 次`],
          ].map(([k, v]) => (
            <div key={k} className="rounded-md bg-surface-2 px-3 py-2">
              <dt className="text-xs text-fg-muted">{k}</dt>
              <dd
                className="amount font-medium"
                data-testid={k === "出价次数" ? "bid-count" : undefined}
              >
                {v}
              </dd>
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

        {live ? (
          <>
            <LiveClock auction={a} notice={notice} />
            {access.kind === "open" ? (
              <BidBox auction={a} place={access.place} fmt={fmt} usdtOf={usdtOf} />
            ) : (
              <div className="rounded-lg bg-surface-2 p-3 text-sm">{access.notice}</div>
            )}
          </>
        ) : (
          <Outcome auction={a} payHref={payHref} signHref={signHref} />
        )}
      </Card>

      <BidHistory bids={bids} fmt={fmt} />
    </div>
  );
}

/** 距结束的倒计时与防狙击说明；进入防狙击窗口后换成警示色。 */
function LiveClock({
  auction: a,
  notice,
}: {
  auction: AuctionPanelData;
  notice?: string | undefined;
}) {
  const snipeMin = Math.round(a.snipeWindowSecs / MINUTE);
  const extendMin = Math.round(a.snipeExtendSecs / MINUTE);
  const sniping = a.secondsLeft > 0 && a.secondsLeft <= a.snipeWindowSecs;
  return (
    <div className="rounded-lg border border-border p-3">
      <p className="flex items-center justify-between gap-2">
        <span className="flex items-center gap-1.5 text-sm text-fg-muted">
          <Hourglass size={15} aria-hidden />
          距结束
        </span>
        <Countdown
          key={a.endsAt}
          seconds={a.secondsLeft}
          ended="结算中…"
          className={cn("text-xl font-semibold", sniping && "text-warning")}
        />
      </p>
      <p className="mt-1 flex gap-1.5 text-xs text-fg-muted">
        <TimerReset size={14} aria-hidden className="mt-0.5 shrink-0" />
        <span>
          结束前 {snipeMin} 分钟内有新出价，自动顺延 {extendMin} 分钟（累计最多{" "}
          {hours(a.maxExtensionSecs)} 小时）
          {a.extendedSecs > 0 ? `；已顺延 ${Math.round(a.extendedSecs / MINUTE)} 分钟` : null}
        </span>
      </p>
      <p role="status" className="mt-1 text-xs font-medium text-warning empty:hidden">
        {notice}
      </p>
    </div>
  );
}

function BidBox({
  auction: a,
  place,
  fmt,
  usdtOf,
}: {
  auction: AuctionPanelData;
  place: (amount: bigint) => Promise<void>;
  fmt: (v: bigint) => string;
  usdtOf: (v: bigint) => string | null;
}) {
  const [draft, setDraft] = useState("");
  const [pending, setPending] = useState<bigint | null>(null);
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const [status, setStatus] = useState("");

  const cur = a.currency;
  const increment = BigInt(a.increment);
  const current = a.current === null ? null : BigInt(a.current);
  const minBid = BigInt(a.minNext);
  const parsed = draft ? parseAuctionAmount(cur, draft) : null;
  const error =
    draft === ""
      ? undefined
      : parsed === null
        ? "金额格式不对"
        : parsed < minBid
          ? `至少出 ${fmt(minBid)}`
          : undefined;
  const minUsdt = usdtOf(minBid);

  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (parsed === null || error) return;
    setFailure(null);
    setStatus("");
    setPending(parsed);
  };
  const confirm = async () => {
    if (pending === null) return;
    setBusy(true);
    try {
      await place(pending);
      setDraft("");
      setStatus("出价成功，你是当前最高出价者。被超价时会通知你（站内 + Telegram）。");
      setPending(null);
    } catch (e) {
      setFailure(e instanceof Error && e.message ? e.message : "出价失败，请稍后再试");
      setPending(null);
    } finally {
      setBusy(false);
    }
  };

  // 快捷加价以当前价为基准；还没人出价时以起拍价为基准，第一档就是起拍价
  const quick = QUICK_STEPS.map((k) => {
    const value = minBid + increment * BigInt(k - 1);
    const label =
      current !== null
        ? `+${fmt(value - current)}`
        : k === 1
          ? "起拍价"
          : `+${fmt(value - minBid)}`;
    return { k, value, label };
  });

  return (
    <>
      {pending === null ? (
        <form onSubmit={submit} className="space-y-2" noValidate aria-label="出价">
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
            className={error ? "text-sm text-danger" : "amount text-sm text-fg-muted"}
            role={error ? "alert" : undefined}
          >
            {error ??
              `最低出价 ${fmt(minBid)}${minUsdt ? `（折合 ${minUsdt}）` : ""}${
                current === null ? "（起拍价）" : "（当前价 + 加价幅度）"
              }`}
          </p>
          <div className="flex gap-2" role="group" aria-label="快捷加价">
            {quick.map((q) => (
              <Button
                key={q.k}
                variant="secondary"
                size="sm"
                className="amount flex-1"
                aria-label={`出价 ${fmt(q.value)}`}
                onClick={() => setDraft(plainAuctionAmount(cur, q.value))}
              >
                {q.label}
              </Button>
            ))}
          </div>
        </form>
      ) : (
        <div className="space-y-2 rounded-lg bg-accent-soft p-3" role="group" aria-label="确认出价">
          <p className="font-medium">确认出价 {fmt(pending)}？</p>
          {usdtOf(pending) && a.cnyPerUsdt !== null ? (
            <p className="amount text-sm" data-testid="bid-usdt">
              成交时货款 {usdtOf(pending)}（按卖家汇率 {formatCnyPerUsdt(a.cnyPerUsdt)}{" "}
              换算，向上取整，与合约一致）
            </p>
          ) : null}
          <p className="text-xs text-fg-muted">
            {EXTRA_COST[a.feePayer] ?? EXTRA_COST[1]}（以钱包付款页为准）。出价不能撤回；成交后需在{" "}
            {hours(a.payWindowSecs)} 小时内付款，逾期记违约并禁拍。
          </p>
          <div className="flex gap-2">
            <Button className="flex-1" onClick={confirm} disabled={busy} autoFocus>
              {busy ? "提交中…" : "确认出价"}
            </Button>
            <Button variant="secondary" onClick={() => setPending(null)} disabled={busy}>
              返回修改
            </Button>
          </div>
        </div>
      )}
      {failure ? (
        <p role="alert" className="text-sm text-danger">
          {failure}
        </p>
      ) : null}
      {/* 被超价后不再显示「你是当前最高出价者」 */}
      <p role="status" className="text-sm text-success empty:hidden">
        {a.my === "leading" ? status : null}
      </p>
    </>
  );
}

/** 拍卖不在进行中：草稿、成交待付款（赢家 / 递补者 / 其他人）、已成交、流拍、已取消。 */
function Outcome({
  auction: a,
  payHref,
  signHref,
}: {
  auction: AuctionPanelData;
  payHref?: string | undefined;
  signHref?: string | undefined;
}) {
  switch (a.state) {
    case "draft":
      return (
        <div className="space-y-2 rounded-lg bg-surface-2 p-3 text-sm">
          <p className="font-medium">等待卖家在钱包签署拍卖授权</p>
          <p className="text-xs text-fg-muted">签名后拍卖才开始，出价入口随之打开。</p>
          {a.my === "seller" && signHref ? (
            <Button asChild className="w-full">
              <a href={signHref}>去钱包签名</a>
            </Button>
          ) : null}
        </div>
      );
    case "ended":
      return (
        <div className="space-y-1 rounded-lg bg-surface-2 p-3 text-sm">
          <p className="font-medium">拍卖已结束，正在结算</p>
          <p className="text-xs text-fg-muted">
            平台核对最高出价者的资格后签发成交证明，或判定流拍；结果出来后本页自动更新。
          </p>
        </div>
      );
    case "awaiting_payment": {
      const payer = a.my === "winner" || a.my === "runner_up_offered";
      const ban = days(a.defaultBanSecs);
      return (
        <div className="space-y-2 rounded-lg bg-accent-soft p-3 text-sm">
          <p className="flex items-center gap-1.5 font-medium">
            <BadgeCheck size={16} aria-hidden className="text-accent" />
            {a.my === "winner"
              ? "你赢得了本场拍卖"
              : a.my === "runner_up_offered"
                ? "原赢家逾期未付款，你可以按自己的出价递补成交"
                : "平台已签发成交证明，等待赢家付款"}
          </p>
          {a.paySecondsLeft !== null ? (
            <p className="flex items-center justify-between gap-2">
              <span className="text-fg-muted">付款截止还剩</span>
              <Countdown seconds={a.paySecondsLeft} className="font-semibold" ended="已逾期" />
            </p>
          ) : null}
          <p className="text-xs text-fg-muted">
            {payer
              ? `在钱包一次 Face ID 付款建单；资金进入担保合约，之后与普通担保交易相同。逾期未付记违约、禁拍 ${ban} 天。`
              : `赢家付款后资金进入担保合约，之后与普通担保交易相同；逾期未付记违约、禁拍 ${ban} 天，由次高出价者递补。`}
          </p>
          {payer && payHref ? (
            <Button asChild className="w-full">
              <a href={payHref}>去付款</a>
            </Button>
          ) : null}
        </div>
      );
    }
    case "settled":
      return (
        <div className="rounded-lg bg-success-soft p-3 text-sm">
          <p className="font-medium text-success">赢家已付款，担保订单已建立</p>
          <p className="mt-1 text-xs text-fg-muted">之后的发货、确认与争议和普通担保交易相同。</p>
        </div>
      );
    case "unsold":
      return (
        <div className="rounded-lg bg-surface-2 p-3 text-sm">
          <p className="font-medium">
            {a.bidCount === 0 ? "无人出价，本场流拍" : "未达保留价，本场流拍"}
          </p>
          <p className="mt-1 text-xs text-fg-muted">
            不建订单、不扣任何费用；卖家可以调整条件后重新挂拍。
          </p>
        </div>
      );
    case "cancelled":
      return (
        <div className="rounded-lg bg-surface-2 p-3 text-sm">
          <p className="font-medium">本场拍卖已取消</p>
        </div>
      );
    case "live":
      return null;
  }
}

function BidHistory({ bids, fmt }: { bids: AuctionPanelBid[]; fmt: (v: bigint) => string }) {
  return (
    <div className="rounded-md border border-border">
      <h2 className="flex items-center justify-between border-b border-border px-4 py-2 text-sm font-medium">
        出价记录
        <span className="text-xs font-normal text-fg-muted">只追加、公开可查 · 出价人已脱敏</span>
      </h2>
      {bids.length === 0 ? (
        <p className="px-4 py-3 text-sm text-fg-muted">还没有人出价</p>
      ) : (
        <ol className="divide-y divide-border text-sm" aria-label="出价记录">
          {bids.map((b, i) => (
            <li key={b.id} className="flex items-center gap-3 px-4 py-2">
              <span className="w-24 truncate">
                {b.bidder}
                {b.mine ? "（你）" : null}
              </span>
              {b.linked ? (
                <Badge tone="neutral" title="与本场其他出价人登录过同一台设备">
                  关联账号
                </Badge>
              ) : null}
              <span className="amount flex-1 font-medium">{fmt(BigInt(b.amount))}</span>
              {i === 0 ? <Badge tone="accent">最高</Badge> : null}
              <time dateTime={b.timeIso} className="amount text-xs text-fg-muted">
                {b.time}
              </time>
            </li>
          ))}
        </ol>
      )}
    </div>
  );
}
