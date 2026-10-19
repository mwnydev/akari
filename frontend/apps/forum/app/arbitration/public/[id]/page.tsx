import { ApiError } from "@akari/api-client";
import { Amount, Badge, Card } from "@akari/ui";
import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { RulingList } from "../../../../components/arbitration/CaseSections";
import { PageError } from "../../../../components/PageError";
import { SiteNav } from "../../../../components/SiteNav";
import { load, serverApi } from "../../../../lib/api";
import { caseStatusLabel } from "../../../../lib/arbitration";
import { formatTime } from "../../../../lib/display";

export const metadata: Metadata = { title: "裁决公示" };

/**
 * 裁决公示（plan 第 7 节：结果和理由对双方公开，脱敏后归入案例库）。接口只给已公示的案件，
 * 不含当事人、仲裁员的身份与钱包地址，所以这一页任何人都能看。
 */
export default async function PublicRulingPage({ params }: { params: Promise<{ id: string }> }) {
  const raw = (await params).id;
  if (!/^\d{1,15}$/.test(raw)) notFound();
  const id = Number(raw);
  const r = await load(
    (await serverApi()).GET("/api/v1/arbitration/public/{id}", { params: { path: { id } } }),
  );
  if (r instanceof ApiError) {
    if (r.status === 404) notFound();
    return (
      <>
        <SiteNav />
        <PageError error={r} />
      </>
    );
  }
  return (
    <>
      <SiteNav />
      <main className="mx-auto max-w-3xl space-y-4 px-4 py-5">
        <h1 className="flex flex-wrap items-center gap-2 text-2xl font-semibold">
          裁决公示 · 案件 #{r.case_id}
          <Badge data-testid="public-status">{caseStatusLabel(r)}</Badge>
        </h1>
        <Card className="space-y-1 text-sm">
          <p>
            {r.mode === "stream" ? "分期担保" : "一次性担保"} · 订单金额 <Amount value={r.amount} />
          </p>
          <p className="text-fg-muted">
            公示于 {r.published_at ? formatTime(r.published_at) : "—"}
            {r.appealed
              ? " · 当事人已申诉，复核结果为最终裁决"
              : r.appeal_until
                ? ` · 申诉期至 ${formatTime(r.appeal_until)}`
                : ""}
          </p>
        </Card>
        <Card className="space-y-3">
          <h2 className="font-semibold">裁决与理由</h2>
          <RulingList rulings={r.rulings} />
        </Card>
        <p className="text-xs text-fg-muted">
          裁决只是意见：链上执行由仲裁 Safe（3 人
          2/3）完成，只能把钱分给买卖双方。本页不显示当事人与仲裁员的身份。
        </p>
      </main>
    </>
  );
}
