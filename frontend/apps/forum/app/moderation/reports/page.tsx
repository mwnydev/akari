import { ApiError, type Schemas } from "@akari/api-client";
import { Badge, Button, EmptyState } from "@akari/ui";
import type { Metadata } from "next";
import Link from "next/link";
import { AppShell } from "../../../components/forum/AppShell";
import { BoardSquare } from "../../../components/forum/BoardMark";
import { ReportActions } from "../../../components/moderation/ReportActions";
import { PageError } from "../../../components/PageError";
import { load, serverApi } from "../../../lib/api";
import { boardColor } from "../../../lib/board-color";
import { cardLookup, formatActivity, formatTime, userName } from "../../../lib/display";
import { listForums, type ForumView } from "../../../lib/forums";
import { topicHref } from "../../../lib/links";
import { floorPage } from "../../../lib/pagination";
import { requireMe } from "../../../lib/session";
import { canHandleReports } from "../../../lib/staff";

export const metadata: Metadata = { title: "举报处理" };

type Search = Promise<Record<string, string | string[] | undefined>>;
type Case = Schemas["ReportCaseView"];

const OUTCOME: Record<string, string> = { dismissed: "未发现违规", actioned: "已处理" };
const ACTION: Record<string, string> = { hide: "隐藏", delete: "删除", sanction: "处罚作者" };

function tabClass(active: boolean) {
  return (
    "border-b-2 px-1 pb-2 text-sm font-medium " +
    (active ? "border-accent text-fg" : "border-transparent text-fg-muted hover:text-fg")
  );
}

function CaseItem({
  c,
  forum,
  user,
  me,
  now,
}: {
  c: Case;
  forum: ForumView | undefined;
  user: ReturnType<typeof cardLookup>;
  me: Schemas["Me"];
  now: Date;
}) {
  const t = c.target;
  const author = user(t.author_id);
  const href = topicHref(t.topic_id, floorPage(t.floor), t.floor);
  return (
    <li className="space-y-2 border-t border-border py-4" data-testid="report-case">
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-sm">
        <Badge tone="outline">{t.kind === "post" ? "楼层" : "评论"}</Badge>
        {forum ? (
          <span className="inline-flex items-center gap-1 text-fg-muted">
            <BoardSquare color={boardColor(forum)} className="size-2.5" />
            {forum.name}
          </span>
        ) : null}
        <Link href={href} className="min-w-0 truncate font-medium hover:underline">
          {t.topic_title} #{t.floor}
        </Link>
        {t.state !== "normal" ? (
          <Badge tone="warning">{t.state === "hidden" ? "已隐藏" : "已删除"}</Badge>
        ) : null}
        <span className="ml-auto text-fg-muted">
          {c.reports} 人举报 · 最近{" "}
          <time dateTime={c.last_reported_at} title={formatTime(c.last_reported_at)}>
            {formatActivity(c.last_reported_at, now)}
          </time>
        </span>
      </div>
      <blockquote className="border-l-2 border-border pl-3 text-sm whitespace-pre-wrap text-fg-muted">
        {t.excerpt || "（无文字内容）"}
      </blockquote>
      <p className="text-sm">
        作者 <span className="font-medium">{userName(author)}</span>
      </p>
      {c.latest.length ? (
        <ul aria-label="举报理由" className="space-y-0.5 text-sm">
          {c.latest.map((r, i) => (
            <li key={i} className="flex gap-2">
              <span className="min-w-0 flex-1">{r.reason || "（未填理由）"}</span>
              <span className="shrink-0 text-fg-muted">
                {r.reporter_id != null ? `${userName(user(r.reporter_id))} · ` : null}
                {formatActivity(r.created_at, now)}
              </span>
            </li>
          ))}
        </ul>
      ) : null}
      {c.status === "open" ? (
        <ReportActions
          caseId={c.id}
          target={{ kind: t.kind, floor: t.floor, forumId: t.forum_id, state: t.state }}
          author={{ id: author.id, name: userName(author) }}
          staff={me.staff ?? null}
        />
      ) : (
        <p className="text-sm text-fg-muted" data-testid="report-outcome">
          {OUTCOME[c.status] ?? c.status}
          {c.action ? `（${ACTION[c.action] ?? c.action}）` : null}
          {c.resolved_by != null ? ` · ${userName(user(c.resolved_by))}` : null}
          {c.resolved_at ? ` · ${formatTime(c.resolved_at)}` : null}
        </p>
      )}
    </li>
  );
}

/**
 * 举报处理（ADR 0013）：同一内容的举报归成一件，版主只看自己的板块，管理员与站长看全部；自己内容上的举报不在这里。
 * 举报人是谁只有管理员与站长看得到。处理方式（隐藏、删除、处罚）走与帖子上同样的权限，结案时通知每位举报人。
 */
export default async function ReportsPage({ searchParams }: { searchParams: Search }) {
  const sp = await searchParams;
  const me = await requireMe("/moderation/reports");
  if (!canHandleReports(me.staff)) {
    return (
      <AppShell>
        <PageError
          title="没有权限"
          error={new ApiError("FORBIDDEN", "举报处理只对版主、管理员和站长开放", 403)}
        />
      </AppShell>
    );
  }
  const resolved = sp.status === "resolved";
  const cursor = typeof sp.cursor === "string" && /^\d{1,18}$/.test(sp.cursor) ? sp.cursor : null;
  const [queue, forums] = await Promise.all([
    load(
      (await serverApi()).GET("/api/v1/moderation/reports", {
        params: {
          query: {
            status: resolved ? "resolved" : "open",
            ...(cursor ? { cursor: Number(cursor) } : {}),
          },
        },
      }),
    ),
    listForums(),
  ]);
  if (queue instanceof ApiError) {
    return (
      <AppShell>
        <PageError error={queue} />
      </AppShell>
    );
  }
  const byId = new Map((forums instanceof ApiError ? [] : forums).map((f) => [f.id, f]));
  const user = cardLookup(queue.users);
  const now = new Date();
  const base = resolved ? "/moderation/reports?status=resolved" : "/moderation/reports";
  return (
    <AppShell>
      <div className="max-w-3xl">
        <h1 className="mb-3 text-2xl font-semibold">举报处理</h1>
        <nav aria-label="举报状态" className="mb-1 flex gap-4 border-b border-border">
          <Link
            href="/moderation/reports"
            aria-current={resolved ? undefined : "page"}
            className={tabClass(!resolved)}
          >
            待处理{queue.open_count ? ` ${queue.open_count}` : ""}
          </Link>
          <Link
            href="/moderation/reports?status=resolved"
            aria-current={resolved ? "page" : undefined}
            className={tabClass(resolved)}
          >
            已处理
          </Link>
        </nav>
        {queue.cases.length ? (
          <ul
            aria-label={resolved ? "已处理的举报" : "待处理的举报"}
            className="border-b border-border"
          >
            {queue.cases.map((c) => (
              <CaseItem
                key={c.id}
                c={c}
                forum={byId.get(c.target.forum_id)}
                user={user}
                me={me}
                now={now}
              />
            ))}
          </ul>
        ) : (
          <EmptyState title={resolved ? "还没有处理过的举报" : "没有待处理的举报"} />
        )}
        {queue.next_cursor ? (
          <p className="py-3 text-center">
            <Button asChild size="sm" variant="secondary">
              <Link href={`${base}${resolved ? "&" : "?"}cursor=${queue.next_cursor}`}>更早的</Link>
            </Button>
          </p>
        ) : null}
      </div>
    </AppShell>
  );
}
