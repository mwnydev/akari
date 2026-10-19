"use client";

import { ApiError, errorMessage } from "@akari/api-client";
import { Badge, Card } from "@akari/ui";
import { useQuery } from "@tanstack/react-query";
import Link from "next/link";
import { caseStatusLabel, RULING_KIND } from "../../lib/arbitration";
import { api } from "../../lib/browser-api";
import { formatTime } from "../../lib/display";
import { bpsLabel } from "../../lib/ruling";
import { QueryProvider } from "../QueryProvider";
import { PartyCaseActions } from "./PartyCaseActions";

/** 订单的仲裁案件；没有案件或无权查看（404）时为 null。 */
async function fetchCase(orderId: number) {
  const { data, error, response } = await api.GET("/api/v1/orders/{id}/case", {
    params: { path: { id: orderId } },
  });
  if (response.status === 404) return null;
  if (!data) throw ApiError.from(error, response.status);
  return data;
}

function Panel({ orderId }: { orderId: number }) {
  const q = useQuery({ queryKey: ["order-case", orderId], queryFn: () => fetchCase(orderId) });
  const c = q.data;
  if (q.isError) {
    return <p className="text-sm text-fg-muted">仲裁案件加载失败：{errorMessage(q.error)}</p>;
  }
  if (!c) return null;
  const effective = c.rulings.find((r) => r.effective);
  return (
    <Card className="space-y-3 text-sm" data-testid="order-case">
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="font-medium">仲裁案件 #{c.id}</h2>
        <Badge tone={c.closed_at ? "neutral" : "warning"}>{caseStatusLabel(c)}</Badge>
      </div>
      <p className="text-fg-muted">
        举证期截止 {formatTime(c.evidence_until)}
        {c.appeal_until ? ` · 申诉期截止 ${formatTime(c.appeal_until)}` : ""}
        {c.appeal_by != null ? " · 已申诉，等待复核" : ""}
      </p>
      {effective ? (
        <p>
          {RULING_KIND[effective.kind]}：本金 {bpsLabel(effective.to_buyer_bps)} 判给买家
          {effective.bond_to_buyer_bps
            ? `，保证金 ${bpsLabel(effective.bond_to_buyer_bps)} 赔给买家`
            : ""}
          。
          <Link href={`/arbitration/public/${c.id}`} className="ml-1 text-accent hover:underline">
            公示页
          </Link>
        </p>
      ) : null}
      <PartyCaseActions
        caseId={c.id}
        statement={c.actions.statement}
        appeal={c.actions.appeal}
        onChange={() => void q.refetch()}
      />
      <Link href={`/arbitration/${c.id}`} className="inline-block text-accent hover:underline">
        查看案件详情、双方陈述与证据
      </Link>
    </Card>
  );
}

/** 订单页里的仲裁入口（只在订单进入过争议时挂载）。 */
export function OrderCasePanel({ orderId }: { orderId: number }) {
  return (
    <QueryProvider>
      <Panel orderId={orderId} />
    </QueryProvider>
  );
}
