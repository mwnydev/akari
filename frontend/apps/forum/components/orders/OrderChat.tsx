"use client";

import { errorMessage, type Schemas } from "@akari/api-client";
import { Badge, ErrorState } from "@akari/ui";
import { ChatBox, type ChatMessage } from "@akari/ui/chat";
import { useQuery } from "@tanstack/react-query";
import { useRouter } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import { api, unwrap } from "../../lib/browser-api";
import { chainBreak, lastId } from "../../lib/chat";
import { formatShortTime } from "../../lib/display";
import { useRealtimeEvents, useTopic } from "../../lib/realtime/react";
import { useMessageStream } from "../chat/useMessageStream";
import { QueryProvider } from "../QueryProvider";

type OrderMessage = Schemas["OrderMessage"];

const POLL_MS = 5_000;
/** 新消息之后多久再请后端重算一次哈希链（连续几条只算一次） */
const VERIFY_DELAY_MS = 1_500;
const TYPING_SHOWN_MS = 4_000;
const TYPING_SEND_EVERY_MS = 2_000;

function shortHash(h: string): string {
  return h.length > 18 ? `${h.slice(0, 10)}…${h.slice(-6)}` : h;
}

/** 哈希链状态：后端重算整条链的结果，加上已加载消息的本地衔接检查。 */
function ChainStatus({ orderId, messages }: { orderId: number; messages: OrderMessage[] }) {
  const verify = useQuery({
    queryKey: ["order-chain", orderId],
    queryFn: () =>
      unwrap(api.GET("/api/v1/orders/{id}/messages/verify", { params: { path: { id: orderId } } })),
  });
  const newest = lastId(messages);
  const refetch = verify.refetch;
  useEffect(() => {
    if (!newest) return;
    const t = setTimeout(() => void refetch(), VERIFY_DELAY_MS);
    return () => clearTimeout(t);
  }, [newest, refetch]);

  const localBreak = chainBreak(messages);
  if (verify.isPending) return <p className="text-xs text-fg-muted">正在校验聊天记录…</p>;
  if (verify.isError) {
    return (
      <p className="text-xs text-fg-muted">聊天记录校验暂不可用：{errorMessage(verify.error)}</p>
    );
  }
  const v = verify.data;
  const broken = v.ok ? localBreak : (v.broken_at ?? null);
  if (!v.ok || broken !== null) {
    return (
      <p role="alert" className="text-xs text-danger" data-testid="chain-status">
        <Badge tone="danger" className="mr-1">
          校验失败
        </Badge>
        {broken !== null ? `第 #${broken} 条起的` : ""}
        记录与哈希链对不上（可能被改动或删除），已无法作为完整证据，请联系仲裁员。
      </p>
    );
  }
  return (
    <p className="text-xs text-fg-muted" data-testid="chain-status">
      <Badge tone="success" className="mr-1">
        记录完整
      </Badge>
      共 {v.count} 条，哈希链校验通过 · 链头{" "}
      <span className="amount">{shortHash(v.head_hash)}</span>
    </p>
  );
}

function Chat({
  orderId,
  me,
  names,
}: {
  orderId: number;
  me: number;
  /** 买卖双方的显示名（仲裁员等其他人显示为「仲裁员」） */
  names: Record<number, string>;
}) {
  const router = useRouter();
  const [reads, setReads] = useState<Record<number, number>>({});
  const [typingUntil, setTypingUntil] = useState(0);
  const [sendError, setSendError] = useState<string | null>(null);
  const lastTypingSent = useRef(0);
  const sendTyping = useTopic({ order_id: orderId });
  const raiseRead = (user: number, id: number) =>
    setReads((r) => (id > (r[user] ?? 0) ? { ...r, [user]: id } : r));

  const stream = useMessageStream<OrderMessage>({
    id: orderId,
    load: async (id, query) => {
      const page = await unwrap(
        api.GET("/api/v1/orders/{id}/messages", { params: { path: { id }, query } }),
      );
      page.reads.forEach((r) => raiseRead(r.user_id, r.last_id));
      return page;
    },
    pick: (e, id) => (e.type === "order_message" && e.message.order_id === id ? e.message : null),
    pollMs: POLL_MS,
  });

  useRealtimeEvents((e) => {
    if (e.type === "order_read" && e.order_id === orderId) raiseRead(e.user_id, e.last_id);
    else if (e.type === "typing" && e.order_id === orderId && e.user_id !== me) {
      setTypingUntil(Date.now() + TYPING_SHOWN_MS);
    } else if (e.type === "order" && e.order_id === orderId) {
      // 状态或确认级别变了：重新渲染服务端的订单摘要
      router.refresh();
    }
  });
  useEffect(() => {
    if (!typingUntil) return;
    const t = setTimeout(() => setTypingUntil(0), Math.max(0, typingUntil - Date.now()));
    return () => clearTimeout(t);
  }, [typingUntil]);

  // 看到了别人的新消息就上报已读
  const newestOther = stream.messages.reduce(
    (max, m) => (m.sender_id !== me && m.id > max ? m.id : max),
    0,
  );
  const myRead = reads[me] ?? 0;
  useEffect(() => {
    if (newestOther <= myRead) return;
    const report = () => {
      if (document.visibilityState !== "visible") return;
      raiseRead(me, newestOther);
      void api.POST("/api/v1/orders/{id}/messages/read", {
        params: { path: { id: orderId } },
        body: { last_id: newestOther },
      });
    };
    report();
    document.addEventListener("visibilitychange", report);
    return () => document.removeEventListener("visibilitychange", report);
  }, [newestOther, myRead, me, orderId]);

  const othersRead = Math.max(
    0,
    ...Object.entries(reads)
      .filter(([u]) => Number(u) !== me)
      .map(([, v]) => v),
  );
  const chat: ChatMessage[] = stream.messages.map((m) => {
    const time = formatShortTime(m.created_at);
    if (m.sender_id == null || m.kind === "system") {
      return { id: String(m.id), kind: "system", text: m.body, time };
    }
    const mine = m.sender_id === me;
    return {
      id: String(m.id),
      kind: mine ? "me" : "them",
      text: m.kind === "image" ? "[图片]" : m.body,
      time,
      warn: m.warn,
      read: mine && m.id <= othersRead,
      author: mine ? undefined : (names[m.sender_id] ?? "仲裁员"),
    };
  });

  if (stream.error) return <ErrorState title="订单聊天加载失败" message={stream.error} />;

  async function send(text: string): Promise<boolean> {
    setSendError(null);
    try {
      const m = await unwrap(
        api.POST("/api/v1/orders/{id}/messages", {
          params: { path: { id: orderId } },
          body: { body: text },
        }),
      );
      stream.add(m);
      return true;
    } catch (e) {
      setSendError(errorMessage(e));
      return false;
    }
  }

  return (
    <ChatBox
      label="订单聊天"
      header={
        <div className="space-y-1">
          <p className="font-medium">订单聊天</p>
          <ChainStatus orderId={orderId} messages={stream.messages} />
        </div>
      }
      messages={chat}
      typing={typingUntil > 0}
      older={stream.older}
      onSend={send}
      onDraftChange={() => {
        const now = Date.now();
        if (now - lastTypingSent.current < TYPING_SEND_EVERY_MS) return;
        lastTypingSent.current = now;
        sendTyping();
      }}
      notice={
        sendError ? (
          <span role="alert" className="text-danger">
            {sendError}
          </span>
        ) : null
      }
    />
  );
}

/** 订单页的聊天（记录只追加、带哈希链，作仲裁证据）。 */
export function OrderChat(props: { orderId: number; me: number; names: Record<number, string> }) {
  return (
    <QueryProvider>
      <Chat {...props} />
    </QueryProvider>
  );
}
