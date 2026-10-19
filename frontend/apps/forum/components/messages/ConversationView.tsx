"use client";

import { ApiError, errorMessage, type Schemas } from "@akari/api-client";
import { Button, ErrorState, LoadingState, UserCard } from "@akari/ui";
import { ChatBox, type ChatMessage } from "@akari/ui/chat";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ShieldCheck } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { api, unwrap } from "../../lib/browser-api";
import { readTarget } from "../../lib/chat";
import { formatShortTime, profileHref, tradeReputation, userName } from "../../lib/display";
import { lastSeenText, setPresence } from "../../lib/presence";
import { WALLET_ORIGIN } from "../../lib/public-config";
import { useRealtimeEvents, useTopic } from "../../lib/realtime/react";
import { usePresence } from "../live/PresenceDot";
import { useMessageStream } from "../chat/useMessageStream";
import { CONVERSATIONS_KEY } from "./ConversationList";
import { OfferMessage, offersKey } from "./OfferMessage";

type Message = Schemas["Message"];

const POLL_MS = 5_000;
/** 对方的「正在输入」显示多久（服务端每 2 秒最多转发一次） */
const TYPING_SHOWN_MS = 4_000;
const TYPING_SEND_EVERY_MS = 2_000;

/** 打开的私信会话：消息（向上翻页）、发送、已读回执、正在输入、对方在线状态、报价卡片、屏蔽。 */
export function ConversationView({ id, me }: { id: number; me: number }) {
  const qc = useQueryClient();
  const meta = useQuery({
    queryKey: ["conversation", id],
    queryFn: () => unwrap(api.GET("/api/v1/conversations/{id}", { params: { path: { id } } })),
  });
  const offers = useQuery({
    queryKey: offersKey(id),
    queryFn: () =>
      unwrap(api.GET("/api/v1/conversations/{id}/offers", { params: { path: { id } } })),
  });
  const stream = useMessageStream<Message>({
    id,
    load: (cid, query) =>
      unwrap(
        api.GET("/api/v1/conversations/{id}/messages", { params: { path: { id: cid }, query } }),
      ),
    pick: (e, cid) =>
      e.type === "message" && e.message.conversation_id === cid ? e.message : null,
    pollMs: POLL_MS,
  });

  const peer = meta.data?.peer;
  const sendTyping = useTopic({ conversation_id: id });
  const presence = usePresence(peer?.id ?? null);
  const [peerReadId, setPeerReadId] = useState(0);
  const [myReadId, setMyReadId] = useState(0);
  const [typingUntil, setTypingUntil] = useState(0);
  const lastTypingSent = useRef(0);
  const [sendError, setSendError] = useState<string | null>(null);

  useEffect(() => {
    if (!meta.data) return;
    setPeerReadId((r) => Math.max(r, meta.data.peer_read_id));
    setMyReadId((r) => Math.max(r, meta.data.my_read_id));
  }, [meta.data]);

  useRealtimeEvents((e) => {
    if (!peer) return;
    if (e.type === "read" && e.conversation_id === id) {
      if (e.user_id === peer.id) setPeerReadId((r) => Math.max(r, e.last_id));
      else setMyReadId((r) => Math.max(r, e.last_id));
    } else if (e.type === "typing" && e.conversation_id === id && e.user_id === peer.id) {
      setTypingUntil(Date.now() + TYPING_SHOWN_MS);
    } else if (e.type === "subscribed" && e.conversation_id === id) {
      e.presence.forEach(setPresence);
    } else if (e.type === "presence" && e.user_id === peer.id) {
      setPresence(e);
    } else if (e.type === "message" && e.message.conversation_id === id) {
      if (e.message.sender_id === peer.id) setTypingUntil(0);
      if (e.message.kind === "offer") void qc.invalidateQueries({ queryKey: offersKey(id) });
    }
  });

  // 「正在输入」到点自动消失
  useEffect(() => {
    if (!typingUntil) return;
    const t = setTimeout(() => setTypingUntil(0), Math.max(0, typingUntil - Date.now()));
    return () => clearTimeout(t);
  }, [typingUntil]);

  // 看到了对方的新消息就上报已读（页面在前台时）
  const target = readTarget(stream.messages, me, myReadId);
  useEffect(() => {
    if (target === null) return;
    const report = () => {
      if (document.visibilityState !== "visible") return;
      setMyReadId((r) => Math.max(r, target));
      // 会话列表的未读数靠服务端推回来的 read 事件清零
      void api.POST("/api/v1/conversations/{id}/read", {
        params: { path: { id } },
        body: { last_id: target },
      });
    };
    report();
    document.addEventListener("visibilitychange", report);
    return () => document.removeEventListener("visibilitychange", report);
  }, [target, id]);

  const block = useMutation({
    mutationFn: (unblock: boolean) => {
      const path = { params: { path: { id: peer?.id ?? 0 } } };
      return unwrap(
        unblock
          ? api.DELETE("/api/v1/users/{id}/block", path)
          : api.POST("/api/v1/users/{id}/block", path),
      );
    },
    onSuccess: () =>
      Promise.all([
        qc.invalidateQueries({ queryKey: ["conversation", id] }),
        qc.invalidateQueries({ queryKey: CONVERSATIONS_KEY }),
      ]),
  });
  const [confirmBlock, setConfirmBlock] = useState(false);

  const offerById = useMemo(
    () => new Map((offers.data ?? []).map((o) => [o.id, o])),
    [offers.data],
  );
  const chat: ChatMessage[] = stream.messages.map((m) => {
    const kind = m.sender_id === me ? "me" : "them";
    const base = {
      id: String(m.id),
      kind,
      time: formatShortTime(m.created_at),
      read: kind === "me" && m.id <= peerReadId,
      warn: m.warn,
    } as const;
    if (m.kind === "offer" && m.offer_id != null) {
      return {
        ...base,
        text: "",
        warn: false,
        content: (
          <OfferMessage
            event={m.body}
            offerId={m.offer_id}
            listed={offerById.get(m.offer_id)}
            listReady={!offers.isPending}
            me={me}
            conversationId={id}
          />
        ),
      };
    }
    return { ...base, text: m.kind === "image" ? "[图片]" : m.body };
  });

  if (meta.isPending) return <LoadingState rows={3} label="正在打开会话" />;
  if (meta.isError) {
    const gone = meta.error instanceof ApiError && meta.error.status === 404;
    return (
      <ErrorState title={gone ? "会话不存在" : "会话加载失败"} message={errorMessage(meta.error)} />
    );
  }
  const c = meta.data;
  const name = userName(c.peer);

  async function send(text: string): Promise<boolean> {
    setSendError(null);
    try {
      const m = await unwrap(
        api.POST("/api/v1/conversations/{id}/messages", {
          params: { path: { id } },
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

  const header = (
    <div className="flex flex-wrap items-center justify-between gap-2">
      <UserCard
        name={name}
        avatar={c.peer.avatar_url ?? undefined}
        level={c.peer.level}
        online={presence?.online}
        lastSeen={presence ? (lastSeenText(presence) ?? undefined) : undefined}
        reputation={tradeReputation(c.peer.reputation)}
        href={profileHref(c.peer)}
        size="sm"
      />
      <div className="flex flex-wrap items-center gap-1">
        {!c.blocked ? (
          <Button asChild size="sm" variant="secondary">
            <a href={`${WALLET_ORIGIN}/offer?conversation=${id}`}>
              <ShieldCheck size={16} aria-hidden />
              发起担保
            </a>
          </Button>
        ) : null}
        {confirmBlock ? (
          <>
            <Button
              size="sm"
              variant="danger"
              disabled={block.isPending}
              onClick={() => block.mutate(false, { onSettled: () => setConfirmBlock(false) })}
            >
              确认屏蔽
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setConfirmBlock(false)}>
              取消
            </Button>
          </>
        ) : (
          <Button
            size="sm"
            variant="ghost"
            disabled={block.isPending}
            onClick={() => (c.blocked ? block.mutate(true) : setConfirmBlock(true))}
          >
            {c.blocked ? "解除屏蔽" : "屏蔽"}
          </Button>
        )}
      </div>
    </div>
  );

  if (stream.error) {
    return (
      <section className="space-y-3">
        {header}
        <ErrorState title="消息加载失败" message={stream.error} />
      </section>
    );
  }

  return (
    <ChatBox
      label={`与 ${name} 的私信`}
      header={header}
      messages={chat}
      typing={typingUntil > 0}
      older={stream.older}
      placeholder={stream.loading ? "正在加载消息…" : `发私信给 ${name}`}
      onSend={send}
      onDraftChange={() => {
        const now = Date.now();
        if (now - lastTypingSent.current < TYPING_SEND_EVERY_MS) return;
        lastTypingSent.current = now;
        sendTyping();
      }}
      notice={
        sendError || block.isError ? (
          <span role="alert" className="text-danger">
            {sendError ?? errorMessage(block.error)}
          </span>
        ) : null
      }
      disabledReason={c.blocked ? "你已屏蔽对方，解除屏蔽后才能互发私信" : undefined}
    />
  );
}
