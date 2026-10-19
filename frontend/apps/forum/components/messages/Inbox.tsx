"use client";

import { Card, EmptyState, cn } from "@akari/ui";
import { ChevronLeft } from "lucide-react";
import Link from "next/link";
import { QueryProvider } from "../QueryProvider";
import { ConversationList } from "./ConversationList";
import { ConversationView } from "./ConversationView";

/** 私信：桌面端左边会话列表、右边打开的会话；手机上二者分页显示。 */
export function Inbox({ me, conversationId }: { me: number; conversationId: number | null }) {
  return (
    <QueryProvider>
      <div className="grid gap-4 lg:grid-cols-[20rem_1fr]">
        <Card
          className={cn("overflow-hidden p-0 sm:p-0", conversationId !== null && "max-lg:hidden")}
        >
          <ConversationList me={me} activeId={conversationId} />
        </Card>
        <div className={cn("min-w-0", conversationId === null && "max-lg:hidden")}>
          {conversationId !== null ? (
            <>
              <Link
                href="/messages"
                className="mb-2 inline-flex items-center gap-1 text-sm text-fg-muted hover:text-fg lg:hidden"
              >
                <ChevronLeft size={16} aria-hidden />
                全部会话
              </Link>
              <ConversationView key={conversationId} id={conversationId} me={me} />
            </>
          ) : (
            <Card>
              <EmptyState
                title="选择一个会话"
                description="消息会实时送达，对方读过后显示「已读」。"
              />
            </Card>
          )}
        </div>
      </div>
    </QueryProvider>
  );
}
