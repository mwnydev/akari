import type { Schemas } from "@akari/api-client";
import { Amount, Badge, Card, UserCard } from "@akari/ui";
import type { ReactNode } from "react";
import { caseTimeline, httpUrl, RULING_KIND } from "../../lib/arbitration";
import { formatTime, profileHref, tradeReputation, userName } from "../../lib/display";
import { ORDER_STATE } from "../../lib/orders";
import { bpsLabel } from "../../lib/ruling";

type Case = Schemas["CaseDetail"];

export function Block({
  title,
  children,
  testId,
}: {
  title: string;
  children: ReactNode;
  testId?: string;
}) {
  return (
    <Card className="space-y-3" data-testid={testId}>
      <h2 className="font-semibold">{title}</h2>
      {children}
    </Card>
  );
}

export function OrderSummary({ c }: { c: Case }) {
  const o = c.order;
  return (
    <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
      <dt className="text-fg-muted">订单</dt>
      <dd>
        #{o.id} · {o.mode === "stream" ? "分期担保" : "一次性担保"} ·{" "}
        {ORDER_STATE[o.state] ?? o.state}
      </dd>
      <dt className="text-fg-muted">金额</dt>
      <dd>
        <Amount value={o.amount} />
      </dd>
      {o.fees.bond ? (
        <>
          <dt className="text-fg-muted">卖家保证金</dt>
          <dd>
            <Amount value={o.fees.bond} />
          </dd>
        </>
      ) : null}
      {o.stream ? (
        <>
          <dt className="text-fg-muted">分期</dt>
          <dd>
            {o.stream.periods} 期 · 已领取 <Amount value={o.stream.claimed} />
          </dd>
        </>
      ) : null}
      <dt className="text-fg-muted">争议发起</dt>
      <dd>
        {o.dispute
          ? `${o.dispute.by === "buyer" ? "买家" : "卖家"} · ${formatTime(o.dispute.disputed_at ?? o.updated_at)}`
          : "—"}
      </dd>
      <dt className="text-fg-muted">链上</dt>
      <dd className="amount break-all">
        链 {o.chain_id} · 合约 {o.contract} · 订单 {o.chain_order_id}
      </dd>
    </dl>
  );
}

export function Parties({ c }: { c: Case }) {
  const party = (
    label: string,
    u: Schemas["UserCardView"] | null | undefined,
    p: Schemas["Party"],
  ) => (
    <div className="space-y-1">
      <p className="text-xs text-fg-muted">{label}</p>
      {u ? (
        <UserCard
          name={userName(u)}
          avatar={u.avatar_url ?? undefined}
          level={u.level}
          reputation={tradeReputation(u.reputation)}
          href={profileHref(u)}
          size="sm"
        />
      ) : (
        <p className="amount text-sm break-all">{p.address}</p>
      )}
    </div>
  );
  return (
    <div className="grid gap-4 sm:grid-cols-2">
      {party("买家", c.buyer, c.order.buyer)}
      {party("卖家", c.seller, c.order.seller)}
    </div>
  );
}

function sideOf(c: Case, userId: number | null | undefined): string {
  if (userId != null && userId === c.order.buyer.user_id) return "买家";
  if (userId != null && userId === c.order.seller.user_id) return "卖家";
  return "系统";
}

export function Statements({ c }: { c: Case }) {
  if (!c.statements.length) return <p className="text-sm text-fg-muted">双方还没有提交陈述。</p>;
  return (
    <ol className="space-y-3">
      {c.statements.map((s) => (
        <li key={s.id} className="space-y-1 text-sm" data-testid="statement">
          <p className="flex items-center gap-2">
            <Badge tone={sideOf(c, s.user_id) === "买家" ? "accent" : "neutral"}>
              {sideOf(c, s.user_id)}
            </Badge>
            <time className="text-xs text-fg-muted" dateTime={s.created_at}>
              {formatTime(s.created_at)}
            </time>
          </p>
          <p className="whitespace-pre-wrap">{s.body}</p>
          {s.uploads.length ? (
            <ul className="flex flex-wrap gap-2">
              {s.uploads.map((u) => {
                const full = httpUrl(u.variants.at(-1)?.url);
                const thumb = httpUrl(u.variants[0]?.url);
                return full && thumb ? (
                  <li key={u.id}>
                    <a href={full} target="_blank" rel="noopener noreferrer">
                      <img src={thumb} alt="证据图片" className="size-20 rounded-md object-cover" />
                    </a>
                  </li>
                ) : null;
              })}
            </ul>
          ) : null}
        </li>
      ))}
    </ol>
  );
}

/** 订单聊天全文（只读）与服务端重算哈希链的结果；链断了不能裁决（服务端拒绝）。 */
export function ChatLog({ c }: { c: Case }) {
  const chat = c.chat;
  if (!chat) return null;
  const v = chat.verification;
  return (
    <div className="space-y-2">
      <p
        className={v.ok ? "text-sm text-success" : "text-sm text-danger"}
        role={v.ok ? undefined : "alert"}
        data-testid="case-chain-status"
      >
        <Badge tone={v.ok ? "success" : "danger"} className="mr-1">
          {v.ok ? "哈希链完整" : "哈希链校验失败"}
        </Badge>
        共 {v.count} 条
        {v.ok
          ? ""
          : `；第 #${v.broken_at ?? "?"} 条起对不上（记录可能被改动），裁决会被拒绝，请联系站长`}
      </p>
      {chat.messages.length ? (
        <ol className="max-h-96 space-y-1.5 overflow-y-auto rounded-md bg-surface-2 p-3 text-sm">
          {chat.messages.map((m) => (
            <li
              key={m.id}
              className={m.kind === "system" ? "text-center text-xs text-fg-muted" : ""}
            >
              {m.kind === "system" ? null : (
                <span className="mr-2 text-xs text-fg-muted">
                  {sideOf(c, m.sender_id)} · {formatTime(m.created_at)}
                </span>
              )}
              {m.kind === "image" && httpUrl(m.image) ? (
                <a
                  href={httpUrl(m.image) ?? undefined}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-accent hover:underline"
                >
                  [图片]
                </a>
              ) : (
                <span className="whitespace-pre-wrap">{m.body}</span>
              )}
              {m.warn ? (
                <Badge tone="warning" className="ml-1">
                  含钱包地址或线下交易说法
                </Badge>
              ) : null}
            </li>
          ))}
        </ol>
      ) : (
        <p className="text-sm text-fg-muted">订单里没有聊天记录。</p>
      )}
    </div>
  );
}

export function Timeline({ c }: { c: Case }) {
  const steps = caseTimeline(c, new Date());
  return (
    <ol className="space-y-1.5 text-sm" data-testid="case-timeline">
      {steps.map((s, i) => (
        <li key={i} className="flex gap-2">
          <span
            aria-hidden
            className={`mt-1.5 size-2 shrink-0 rounded-full ${s.done ? "bg-accent" : "border border-border"}`}
          />
          <span className={s.done ? "" : "text-fg-muted"}>
            {s.label}
            {s.at ? <span className="ml-2 text-xs text-fg-muted">{formatTime(s.at)}</span> : null}
          </span>
        </li>
      ))}
    </ol>
  );
}

export function RulingList({ rulings }: { rulings: Schemas["CaseRuling"][] }) {
  return (
    <ol className="space-y-3">
      {rulings.map((r) => (
        <li key={r.id} className="space-y-1 text-sm" data-testid="ruling">
          <p className="flex flex-wrap items-center gap-2">
            <Badge tone="outline">{RULING_KIND[r.kind]}</Badge>
            {r.effective ? <Badge tone="success">当前生效</Badge> : null}
            <time className="text-xs text-fg-muted" dateTime={r.created_at}>
              {formatTime(r.created_at)}
            </time>
          </p>
          <p>
            本金 {bpsLabel(r.to_buyer_bps)} 判给买家：买家 <Amount value={r.principal_to_buyer} /> ·
            卖家 <Amount value={r.principal_to_seller} />
            {r.bond_to_buyer_bps ? (
              <>
                {" "}
                · 保证金 {bpsLabel(r.bond_to_buyer_bps)} 赔给买家 <Amount value={r.bond_to_buyer} />
              </>
            ) : null}
          </p>
          <p className="whitespace-pre-wrap text-fg-muted">理由：{r.reason}</p>
        </li>
      ))}
    </ol>
  );
}
