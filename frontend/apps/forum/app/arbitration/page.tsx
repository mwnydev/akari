import { ApiError, type Schemas } from "@akari/api-client";
import { Badge, Card, EmptyState } from "@akari/ui";
import type { Metadata } from "next";
import Link from "next/link";
import { RulingPasskeyCard } from "../../components/arbitration/RulingPasskeyCard";
import { PageError } from "../../components/PageError";
import { SiteNav } from "../../components/SiteNav";
import { load, serverApi } from "../../lib/api";
import { CASE_STATUS } from "../../lib/arbitration";
import { formatTime } from "../../lib/display";
import { requireMe } from "../../lib/session";
import { isArbitrator } from "../../lib/staff";

export const metadata: Metadata = { title: "仲裁待办" };

const ROLE: Record<string, string> = { panel: "裁决", appeal: "申诉复核" };

function CaseLink({ id }: { id: number }) {
  return (
    <Link href={`/arbitration/${id}`} className="font-medium text-accent hover:underline">
      案件 #{id}
    </Link>
  );
}

function Mine({ items }: { items: Schemas["QueueEntry"][] }) {
  if (!items.length) return <EmptyState title="没有分配给你的案件" />;
  return (
    <ul className="divide-y divide-border">
      {items.map((q) => (
        <li
          key={`${q.case_id}-${q.role}`}
          className="flex flex-wrap items-center gap-2 py-2 text-sm"
          data-testid="queue-item"
        >
          <CaseLink id={q.case_id} />
          <Badge tone="outline">{ROLE[q.role] ?? q.role}</Badge>
          <Badge>{CASE_STATUS[q.status] ?? q.status}</Badge>
          <span className="ml-auto text-fg-muted">
            举证截止 {formatTime(q.evidence_until)} · 裁决期限 {formatTime(q.due_at)}
          </span>
        </li>
      ))}
    </ul>
  );
}

/** 仲裁待办（管理员与站长，ADR 0013 第 2 条）：分配给我的案件；站长另看需要终裁、复核、叫停或执行的案件。 */
export default async function ArbitrationQueuePage() {
  const me = await requireMe("/arbitration");
  if (!isArbitrator(me.staff)) {
    return (
      <>
        <SiteNav />
        <PageError
          title="没有权限"
          error={new ApiError("FORBIDDEN", "仲裁页面只对管理员和站长开放", 403)}
        />
      </>
    );
  }
  const queue = await load((await serverApi()).GET("/api/v1/arbitration/queue"));
  if (queue instanceof ApiError) {
    return (
      <>
        <SiteNav />
        <PageError error={queue} />
      </>
    );
  }
  return (
    <>
      <SiteNav />
      <main className="mx-auto max-w-4xl space-y-4 px-4 py-5">
        <h1 className="text-2xl font-semibold">仲裁待办</h1>
        <RulingPasskeyCard />
        <Card className="space-y-2">
          <h2 className="font-semibold">分配给我的</h2>
          <Mine items={queue.assignments} />
        </Card>
        {me.staff?.roles.includes("owner") ? (
          <Card className="space-y-2">
            <h2 className="font-semibold">站长待办</h2>
            {queue.owner.length ? (
              <ul className="divide-y divide-border">
                {queue.owner.map((o) => (
                  <li key={o.case_id} className="flex items-center gap-2 py-2 text-sm">
                    <CaseLink id={o.case_id} />
                    <Badge tone="warning">{CASE_STATUS[o.status] ?? o.status}</Badge>
                  </li>
                ))}
              </ul>
            ) : (
              <EmptyState title="没有需要站长处理的案件" />
            )}
          </Card>
        ) : null}
        <p className="text-xs text-fg-muted">
          与当事人有关联或 90 天内有交易的案件会自动回避；计酬只按工作量与按时率，与裁决方向无关。
        </p>
      </main>
    </>
  );
}
