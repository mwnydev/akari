"use client";

import { errorMessage, type Schemas } from "@akari/api-client";
import { Avatar, Badge, Button, EmptyState, ErrorState, LoadingState, cn } from "@akari/ui";
import { useInfiniteQuery, useQueryClient, type InfiniteData } from "@tanstack/react-query";
import Link from "next/link";
import { api, unwrap } from "../../lib/browser-api";
import { badgeText } from "../../lib/badge";
import { applyMessage, applyRead } from "../../lib/chat";
import { formatShortTime, userName } from "../../lib/display";
import { useRealtimeEvents, useRealtimeStatus } from "../../lib/realtime/react";
import { PresenceDot } from "../live/PresenceDot";
import { messagePreview } from "./preview";

type Page = Schemas["ConversationPage"];
type Conversation = Schemas["ConversationView"];

export const CONVERSATIONS_KEY = ["conversations"] as const;
const POLL_MS = 15_000;

/** 推送更新后把各页合成一页（保留最后一页的游标，「加载更多」照常接着翻）。 */
function rewrite(
  data: InfiniteData<Page, number | null>,
  list: Conversation[],
): InfiniteData<Page, number | null> {
  const last = data.pages.at(-1);
  return {
    pages: [{ conversations: list, next_cursor: last?.next_cursor ?? null }],
    pageParams: [data.pageParams[0] ?? null],
  };
}

/** 会话列表：最近的在前，显示未读数与对方在线状态；新私信与已读回执实时更新。 */
export function ConversationList({ me, activeId }: { me: number; activeId: number | null }) {
  const qc = useQueryClient();
  const status = useRealtimeStatus();
  const q = useInfiniteQuery({
    queryKey: CONVERSATIONS_KEY,
    queryFn: ({ pageParam }) =>
      unwrap(
        api.GET("/api/v1/conversations", {
          params: { query: pageParam === null ? {} : { cursor: pageParam } },
        }),
      ),
    initialPageParam: null as number | null,
    getNextPageParam: (last) => last.next_cursor ?? undefined,
    refetchInterval: status === "polling" ? POLL_MS : false,
  });

  useRealtimeEvents((e) => {
    if (e.type === "resync") {
      void qc.invalidateQueries({ queryKey: CONVERSATIONS_KEY });
      return;
    }
    if (e.type !== "message" && e.type !== "read") return;
    const data = qc.getQueryData<InfiniteData<Page, number | null>>(CONVERSATIONS_KEY);
    if (!data) return;
    const flat = data.pages.flatMap((p) => p.conversations);
    const next =
      e.type === "message" ? applyMessage(flat, e.message, me, activeId) : applyRead(flat, e, me);
    if (next === null) void qc.invalidateQueries({ queryKey: CONVERSATIONS_KEY });
    else qc.setQueryData(CONVERSATIONS_KEY, rewrite(data, next));
  });

  if (q.isPending) return <LoadingState rows={4} label="正在加载会话" />;
  if (q.isError) {
    return (
      <ErrorState
        title="会话加载失败"
        message={errorMessage(q.error)}
        action={
          <Button variant="secondary" size="sm" onClick={() => void q.refetch()}>
            重试
          </Button>
        }
      />
    );
  }
  const list = q.data.pages.flatMap((p) => p.conversations);
  if (!list.length) {
    return (
      <EmptyState
        title="还没有私信"
        description="在别人的个人主页点「发私信」开始聊天；担保交易也从私信里发起。"
      />
    );
  }
  return (
    <div>
      <ul className="divide-y divide-border" aria-label="会话列表">
        {list.map((c) => {
          const name = userName(c.peer);
          const badge = badgeText(c.unread);
          return (
            <li key={c.id}>
              <Link
                href={`/messages/${c.id}`}
                aria-current={c.id === activeId ? "page" : undefined}
                data-testid="conversation"
                className={cn(
                  "flex items-center gap-3 px-4 py-3 hover:bg-surface-2",
                  c.id === activeId && "bg-accent-soft",
                )}
              >
                <Avatar
                  name={name}
                  src={c.peer.avatar_url ?? undefined}
                  size={40}
                  badge={<PresenceDot userId={c.peer.id} avatarSize={40} />}
                />
                <span className="min-w-0 flex-1">
                  <span className="flex items-baseline justify-between gap-2">
                    <span className="truncate font-medium">{name}</span>
                    {c.last_message ? (
                      <time
                        dateTime={c.last_message.created_at}
                        className="shrink-0 text-xs text-fg-muted"
                      >
                        {formatShortTime(c.last_message.created_at)}
                      </time>
                    ) : null}
                  </span>
                  <span className="flex items-center justify-between gap-2">
                    <span className="truncate text-sm text-fg-muted">
                      {c.blocked ? "已屏蔽 · " : ""}
                      {c.last_message ? messagePreview(c.last_message, me) : "还没有消息"}
                    </span>
                    {badge ? (
                      <Badge tone="danger" aria-label={`${badge} 条未读`} data-testid="unread">
                        {badge}
                      </Badge>
                    ) : null}
                  </span>
                </span>
              </Link>
            </li>
          );
        })}
      </ul>
      {q.hasNextPage ? (
        <div className="p-3 text-center">
          <Button
            variant="secondary"
            size="sm"
            disabled={q.isFetchingNextPage}
            onClick={() => void q.fetchNextPage()}
          >
            {q.isFetchingNextPage ? "加载中…" : "加载更多会话"}
          </Button>
        </div>
      ) : null}
    </div>
  );
}
