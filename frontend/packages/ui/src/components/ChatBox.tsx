"use client";

import { SendHorizontal, TriangleAlert } from "lucide-react";
import { useState, type ReactNode } from "react";
import { cn } from "../cn";

export interface ChatMessage {
  id: string;
  /** system = 付款、发货、和解提议等自动插入的系统消息 */
  kind: "me" | "them" | "system";
  text: string;
  time: string;
  read?: boolean;
}

/** 钱包地址或「线下 / 私下转账」：提示只走担保（plan 第 7 节） */
const OFF_PLATFORM = /0x[0-9a-fA-F]{40}|线下转账|私下转账|线下交易/;

export function ChatBox({
  header,
  messages: initial,
  typing = false,
  readOnly = false,
}: {
  header?: ReactNode;
  messages: ChatMessage[];
  typing?: boolean;
  readOnly?: boolean;
}) {
  const [messages, setMessages] = useState(initial);
  const [draft, setDraft] = useState("");
  const warn =
    messages.some((m) => m.kind !== "system" && OFF_PLATFORM.test(m.text)) ||
    OFF_PLATFORM.test(draft);

  function send() {
    const text = draft.trim();
    if (!text) return;
    setMessages((m) => [...m, { id: `local-${m.length}`, kind: "me", text, time: "刚刚" }]);
    setDraft("");
  }

  return (
    <section
      className="flex min-w-0 flex-col overflow-hidden rounded-xl border border-border bg-surface"
      aria-label="订单聊天"
    >
      {header ? <div className="border-b border-border px-4 py-3">{header}</div> : null}
      <ol className="flex max-h-[28rem] min-h-64 flex-col gap-2 overflow-y-auto px-3 py-4">
        {messages.map((m) =>
          m.kind === "system" ? (
            <li
              key={m.id}
              className="mx-auto max-w-[90%] rounded-pill bg-surface-2 px-3 py-1 text-center text-xs text-fg-muted"
            >
              {m.text} · {m.time}
            </li>
          ) : (
            <li
              key={m.id}
              className={cn(
                "flex max-w-[80%] flex-col",
                m.kind === "me" ? "items-end self-end" : "items-start",
              )}
            >
              <span
                className={cn(
                  "rounded-lg px-3 py-2 break-words",
                  m.kind === "me"
                    ? "rounded-br-sm bg-accent-fill text-accent-fg"
                    : "rounded-bl-sm bg-surface-2",
                )}
              >
                {m.text}
              </span>
              <span className="mt-0.5 text-xs text-fg-muted">
                {m.time}
                {m.kind === "me" && m.read ? " · 已读" : ""}
              </span>
            </li>
          ),
        )}
        {typing ? <li className="text-xs text-fg-muted">对方正在输入…</li> : null}
      </ol>
      {warn ? (
        <p className="flex items-start gap-2 border-t border-border bg-warning-soft px-4 py-2 text-sm text-warning">
          <TriangleAlert size={16} className="mt-1 shrink-0" aria-hidden />
          请只通过担保交易付款。要求线下转账或直接转到钱包地址的，多为诈骗。
        </p>
      ) : null}
      {readOnly ? (
        <p className="border-t border-border px-4 py-3 text-center text-sm text-fg-muted">
          聊天记录只读，已作为仲裁证据保存
        </p>
      ) : (
        <form
          className="flex items-center gap-2 border-t border-border p-2"
          onSubmit={(e) => {
            e.preventDefault();
            send();
          }}
        >
          <input
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            placeholder="发消息（记录永久保存，作为仲裁证据）"
            aria-label="消息内容"
            className="h-10 min-w-0 flex-1 rounded-pill bg-surface-2 px-4 placeholder:text-fg-muted focus-visible:outline-2 focus-visible:outline-accent"
          />
          <button
            type="submit"
            className="inline-flex h-10 shrink-0 items-center gap-1 rounded-pill bg-accent-fill px-4 text-accent-fg disabled:opacity-45"
            disabled={!draft.trim()}
          >
            <SendHorizontal size={18} aria-hidden />
            发送
          </button>
        </form>
      )}
    </section>
  );
}
