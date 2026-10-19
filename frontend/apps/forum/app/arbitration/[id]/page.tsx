import { ApiError } from "@akari/api-client";
import { Badge, Card } from "@akari/ui";
import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";
import {
  Block,
  ChatLog,
  OrderSummary,
  Parties,
  RulingList,
  Statements,
  Timeline,
} from "../../../components/arbitration/CaseSections";
import { ExecutionDownload } from "../../../components/arbitration/ExecutionDownload";
import {
  LazyCasePartyActions as CasePartyActions,
  LazyOwnerControls as OwnerControls,
  LazyRulingForm as RulingForm,
} from "../../../components/arbitration/lazy";
import { PageError } from "../../../components/PageError";
import { SiteNav } from "../../../components/SiteNav";
import { load, serverApi } from "../../../lib/api";
import { caseStatusLabel, RULING_KIND, SEAT_LABEL } from "../../../lib/arbitration";
import { formatTime } from "../../../lib/display";
import { requireMe } from "../../../lib/session";

export const metadata: Metadata = { title: "案件" };

/**
 * 案件详情：当事人、本案仲裁员与站长可见（其他人 404，不暴露案件是否存在）。可见范围由服务端过滤——
 * 两人盲审时仲裁员只拿到自己的裁决，当事人在公示前拿不到任何裁决，任何人都拿不到仲裁员身份。
 */
export default async function CasePage({ params }: { params: Promise<{ id: string }> }) {
  const raw = (await params).id;
  if (!/^\d{1,15}$/.test(raw)) notFound();
  const id = Number(raw);
  await requireMe(`/arbitration/${id}`);
  const c = await load(
    (await serverApi()).GET("/api/v1/arbitration/cases/{id}", { params: { path: { id } } }),
  );
  if (c instanceof ApiError) {
    if (c.status === 404) notFound();
    return (
      <>
        <SiteNav />
        <PageError error={c} />
      </>
    );
  }
  const seat = c.actions.seat;
  const party = seat === "buyer" || seat === "seller";
  const open = !c.closed_at;
  const rule = open ? c.actions.rule : null;
  const evidenceOpen = new Date(c.evidence_until) > new Date();
  const blind = seat === "panel" && c.panel_size === 2 && c.status === "reviewing";

  return (
    <>
      <SiteNav />
      <main className="mx-auto grid max-w-6xl gap-4 px-4 py-5 lg:grid-cols-[1fr_22rem]">
        <section className="min-w-0 space-y-4">
          <div className="space-y-1">
            <p className="text-sm text-fg-muted">
              <Link
                href={party ? `/orders/${c.order.id}` : "/arbitration"}
                className="hover:text-fg"
              >
                {party ? `订单 #${c.order.id}` : "仲裁待办"}
              </Link>{" "}
              / 案件 #{c.id}
            </p>
            <h1 className="flex flex-wrap items-center gap-2 text-2xl font-semibold">
              仲裁案件 #{c.id}
              <Badge tone={open ? "warning" : "neutral"} data-testid="case-status">
                {caseStatusLabel(c)}
              </Badge>
              {c.panel_size === 2 ? <Badge tone="outline">≥ 200 USDT · 两人独立裁决</Badge> : null}
            </h1>
            <p className="text-sm text-fg-muted">你在本案中是：{SEAT_LABEL[seat] ?? seat}</p>
          </div>

          {c.halt ? (
            <Card className="text-sm text-warning" role="status">
              已叫停：{c.halt.reason ?? "未写理由"}
            </Card>
          ) : null}

          {c.appeal_by != null ? (
            <Block title="申诉">
              <p className="text-sm whitespace-pre-wrap">
                {c.appeal_by === c.order.buyer.user_id ? "买家" : "卖家"}申诉：
                {c.appeal_reason ?? ""}
              </p>
              <p className="text-xs text-fg-muted">由没参与过本案的人复核，复核结果为最终裁决。</p>
            </Block>
          ) : null}

          <Block title="双方陈述与证据" testId="case-statements">
            <Statements c={c} />
            {party && open ? (
              <CasePartyActions
                caseId={c.id}
                statement={c.actions.statement}
                appeal={c.actions.appeal}
              />
            ) : null}
          </Block>

          {c.chat ? (
            <Block title="订单聊天（只读，哈希链校验）" testId="case-chat">
              <ChatLog c={c} />
            </Block>
          ) : null}

          <Block title="裁决" testId="case-rulings">
            {blind ? (
              <p className="text-sm text-fg-muted">
                两人独立裁决：另一位仲裁员是谁、交没交、判了什么，在两人都提交之前都不显示。
              </p>
            ) : null}
            {c.rulings.length ? (
              <RulingList rulings={c.rulings} />
            ) : (
              <p className="text-sm text-fg-muted">
                {party ? "裁决公示后在这里显示（结果与理由对双方公开）。" : "还没有裁决。"}
              </p>
            )}
            {c.published_at ? (
              <p className="text-sm">
                公示于 {formatTime(c.published_at)}
                {c.appeal_until ? ` · 申诉期至 ${formatTime(c.appeal_until)}` : ""} ·{" "}
                <Link href={`/arbitration/public/${c.id}`} className="text-accent hover:underline">
                  公示页
                </Link>
              </p>
            ) : null}
          </Block>

          {rule ? (
            <Block title={`提交${RULING_KIND[rule]}`} testId="ruling-form">
              <RulingForm caseId={c.id} kind={rule} order={c.order} evidenceOpen={evidenceOpen} />
            </Block>
          ) : null}

          {c.actions.halt || c.actions.resume || c.execution ? (
            <Block title="执行">
              {c.actions.halt || c.actions.resume ? (
                <OwnerControls caseId={c.id} halt={c.actions.halt} resume={c.actions.resume} />
              ) : null}
              {c.execution ? (
                <div className="space-y-1 text-sm">
                  <p>
                    待仲裁 Safe 执行{" "}
                    <span className="amount">
                      resolve({c.execution.chain_order_id}, {c.execution.to_buyer_bps},{" "}
                      {c.execution.bond_to_buyer_bps})
                    </span>
                  </p>
                  <ExecutionDownload execution={c.execution} />
                </div>
              ) : null}
            </Block>
          ) : null}
        </section>

        <aside className="min-w-0 space-y-4">
          <Block title="订单摘要">
            <OrderSummary c={c} />
          </Block>
          <Block title="当事人">
            <Parties c={c} />
          </Block>
          <Block title="进度">
            <Timeline c={c} />
          </Block>
        </aside>
      </main>
    </>
  );
}
