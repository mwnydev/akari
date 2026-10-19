"use client";

import { errorMessage, type Schemas } from "@akari/api-client";
import { Button, Card, EmptyState, ErrorState, LoadingState, cn } from "@akari/ui";
import {
  useInfiniteQuery,
  useQuery,
  useQueryClient,
  type InfiniteData,
} from "@tanstack/react-query";
import Link from "next/link";
import { useCallback, useEffect, useState } from "react";
import { api, unwrap } from "../../lib/browser-api";
import { lastId, mergeNewestFirst } from "../../lib/chat";
import { formatShortTime, userName } from "../../lib/display";
import { describeNotification } from "../../lib/notifications";
import { broadcastLocal, useRealtimeEvents, useRealtimeStatus } from "../../lib/realtime/react";

type Page = Schemas["NotificationPage"];
type Data = InfiniteData<Page, number | null>;

const KEY = ["notifications"] as const;
const POLL_MS = 30_000;
/** 补拉最多连拉几页（每页 20 条） */
const MAX_CATCH_UP_PAGES = 10;

/** 新通知（推送或补拉）插到第一页最前面。 */
function prepend(data: Data | undefined, incoming: Schemas["Notification"][]): Data | undefined {
  if (!data || !incoming.length) return data;
  const [first, ...rest] = data.pages;
  if (!first) return data;
  return {
    ...data,
    pages: [{ ...first, notifications: mergeNewestFirst(first.notifications, incoming) }, ...rest],
  };
}

/** 通知列表：新的在前、未读高亮；推送实时插入，重连或轮询时用 `after` 补拉；「全部标为已读」同步到其他标签页。 */
export function NotificationList() {
  const qc = useQueryClient();
  const status = useRealtimeStatus();
  const [readMark, setReadMark] = useState(0);
  const q = useInfiniteQuery({
    queryKey: KEY,
    queryFn: ({ pageParam }) =>
      unwrap(
        api.GET("/api/v1/notifications", {
          params: { query: pageParam === null ? {} : { before: pageParam } },
        }),
      ),
    initialPageParam: null as number | null,
    getNextPageParam: (last) => last.next_cursor ?? undefined,
  });
  const list = q.data?.pages.flatMap((p) => p.notifications) ?? [];
  const lastRead = Math.max(readMark, q.data?.pages[0]?.last_read_id ?? 0);

  const catchUp = useCallback(async () => {
    const data = qc.getQueryData<Data>(KEY);
    if (!data) return;
    let after = lastId(data.pages.flatMap((p) => p.notifications));
    try {
      for (let i = 0; i < MAX_CATCH_UP_PAGES; i++) {
        const page = await unwrap(
          api.GET("/api/v1/notifications", { params: { query: { after } } }),
        );
        qc.setQueryData<Data>(KEY, (d) => prepend(d, page.notifications));
        setReadMark((m) => Math.max(m, page.last_read_id));
        if (!page.next_cursor) break;
        after = page.next_cursor;
      }
    } catch {
      // 下次重连或轮询再补
    }
  }, [qc]);

  useRealtimeEvents((e) => {
    if (e.type === "notification") qc.setQueryData<Data>(KEY, (d) => prepend(d, [e.notification]));
    else if (e.type === "notifications_read") setReadMark((m) => Math.max(m, e.last_read_id));
    else if (e.type === "resync") void catchUp();
  });
  useEffect(() => {
    if (status !== "polling" || !q.isSuccess) return;
    const t = setInterval(() => void catchUp(), POLL_MS);
    return () => clearInterval(t);
  }, [status, q.isSuccess, catchUp]);

  const actorIds = [...new Set(list.flatMap((n) => (n.actor_id != null ? [n.actor_id] : [])))]
    .sort((a, b) => a - b)
    .slice(0, 100);
  const cards = useQuery({
    queryKey: ["user-cards", actorIds.join(",")],
    queryFn: () =>
      unwrap(api.GET("/api/v1/users/cards", { params: { query: { ids: actorIds.join(",") } } })),
    enabled: actorIds.length > 0,
    staleTime: 5 * 60_000,
  });
  const nameOf = (id: number | null | undefined) => {
    if (id == null) return null;
    const card = cards.data?.find((c) => c.id === id);
    return userName(card ?? { id });
  };

  const [marking, setMarking] = useState<string | null>(null);
  async function markAll() {
    const top = lastId(list);
    if (!top) return;
    setMarking(null);
    try {
      const res = await unwrap(api.POST("/api/v1/notifications/read", { body: { up_to_id: top } }));
      setReadMark((m) => Math.max(m, res.last_read_id));
      broadcastLocal({
        type: "notifications_read",
        last_read_id: res.last_read_id,
        count: res.count,
      });
    } catch (e) {
      setMarking(errorMessage(e));
    }
  }

  if (q.isPending) return <LoadingState rows={4} label="正在加载通知" />;
  if (q.isError) {
    return (
      <Card>
        <ErrorState title="通知加载失败" message={errorMessage(q.error)} />
      </Card>
    );
  }
  const hasUnread = list.some((n) => n.id > lastRead);
  return (
    <Card className="p-0 sm:p-0">
      <div className="flex items-center justify-between gap-2 px-4 pt-4 pb-2">
        <h1 className="text-lg font-semibold">通知</h1>
        <Button variant="secondary" size="sm" disabled={!hasUnread} onClick={() => void markAll()}>
          全部标为已读
        </Button>
      </div>
      {marking ? (
        <p role="alert" className="px-4 pb-2 text-sm text-danger">
          {marking}
        </p>
      ) : null}
      {list.length ? (
        <ul className="divide-y divide-border" aria-label="通知列表">
          {list.map((n) => {
            const view = describeNotification(n, nameOf(n.actor_id));
            const unread = n.id > lastRead;
            const body = (
              <>
                <span className="flex items-center gap-2">
                  {unread ? (
                    <span className="size-2 shrink-0 rounded-full bg-accent" aria-label="未读" />
                  ) : null}
                  <span className={cn(unread && "font-medium")}>{view.text}</span>
                </span>
                <time dateTime={n.created_at} className="text-xs text-fg-muted">
                  {formatShortTime(n.created_at)}
                </time>
              </>
            );
            return (
              <li key={n.id} data-testid="notification" data-unread={unread || undefined}>
                {view.href ? (
                  <Link
                    href={view.href}
                    className="flex flex-col gap-0.5 px-4 py-3 hover:bg-surface-2"
                  >
                    {body}
                  </Link>
                ) : (
                  <div className="flex flex-col gap-0.5 px-4 py-3">{body}</div>
                )}
              </li>
            );
          })}
        </ul>
      ) : (
        <EmptyState
          title="还没有通知"
          description="有人回复、@你、发私信或订单有变化时会出现在这里。"
        />
      )}
      {q.hasNextPage ? (
        <div className="p-3 text-center">
          <Button
            variant="secondary"
            size="sm"
            disabled={q.isFetchingNextPage}
            onClick={() => void q.fetchNextPage()}
          >
            {q.isFetchingNextPage ? "加载中…" : "更早的通知"}
          </Button>
        </div>
      ) : null}
    </Card>
  );
}
