"use client";

import { SendHorizontal, TriangleAlert } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { cn } from "../cn";

export interface ChatMessage {
  id: string;
  /** system = 付款、发货、和解提议等自动插入的系统消息 */
  kind: "me" | "them" | "system";
  text: string;
  time: string;
  read?: boolean;
  /** 服务端给出的风险标记（含钱包地址或线下交易说法）；不给时按正文本地判断 */
  warn?: boolean;
  /** 替代文字气泡的内容（如私信里的报价卡片） */
  content?: ReactNode;
  /** 气泡上方的发言人（订单里仲裁员也会发言） */
  author?: string | undefined;
}

/** 向上翻页：滚到顶部时自动加载更早的消息。 */
export interface OlderMessages {
  hasMore: boolean;
  loading: boolean;
  load: () => void;
}

export interface ChatBoxProps {
  header?: ReactNode | undefined;
  messages: ChatMessage[];
  typing?: boolean | undefined;
  readOnly?: boolean | undefined;
  /** 整个聊天区域的无障碍名称 */
  label?: string | undefined;
  placeholder?: string | undefined;
  /**
   * 发送（受控模式）：返回 true 表示已发出、清空输入框，消息列表由调用方更新。
   * 不传时为本地演示模式（设计系统页、模拟数据页面）。
   */
  onSend?: ((text: string) => Promise<boolean> | boolean) | undefined;
  /** 输入框内容变化（发「正在输入」用） */
  onDraftChange?: ((text: string) => void) | undefined;
  older?: OlderMessages | undefined;
  /** 输入框上方的提示（发送失败等） */
  notice?: ReactNode | undefined;
  /** 不能发言时的说明（如已屏蔽），替代输入框 */
  disabledReason?: string | undefined;
  maxLength?: number | undefined;
}

/** 钱包地址或「线下 / 私下转账」：提示只走担保（plan 第 7 节） */
const OFF_PLATFORM = /0x[0-9a-fA-F]{40}|线下转账|私下转账|线下交易/;
/** 离底部这么近（px）时，新消息到来自动滚到底 */
const STICK_TO_BOTTOM = 80;

export function ChatBox({
  header,
  messages: given,
  typing = false,
  readOnly = false,
  label = "订单聊天",
  placeholder = "发消息（记录永久保存，作为仲裁证据）",
  onSend,
  onDraftChange,
  older,
  notice,
  disabledReason,
  maxLength = 2000,
}: ChatBoxProps) {
  const [local, setLocal] = useState(given);
  const messages = onSend ? given : local;
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const listRef = useRef<HTMLOListElement>(null);
  const topRef = useRef<HTMLLIElement>(null);
  // 上一次渲染时列表的首尾与滚动位置：区分「向上加载了更早的」与「底部来了新消息」
  const snapshot = useRef<{
    first?: string | undefined;
    last?: string | undefined;
    height: number;
    top: number;
  }>({
    height: 0,
    top: 0,
  });
  const warn =
    messages.some(
      (m) => m.kind !== "system" && (m.warn ?? (!m.content && OFF_PLATFORM.test(m.text))),
    ) || OFF_PLATFORM.test(draft);

  useLayoutEffect(() => {
    const el = listRef.current;
    if (!el) return;
    const prev = snapshot.current;
    const first = messages[0]?.id;
    const last = messages.at(-1)?.id;
    if (prev.first !== undefined && first !== prev.first && last === prev.last) {
      // 更早的消息插到了上面：保持当前看到的位置不动
      el.scrollTop = el.scrollHeight - prev.height + prev.top;
    } else if (last !== prev.last) {
      const nearBottom = prev.height - prev.top - el.clientHeight < STICK_TO_BOTTOM;
      if (prev.last === undefined || nearBottom || messages.at(-1)?.kind === "me") {
        el.scrollTop = el.scrollHeight;
      }
    }
    snapshot.current = { first, last, height: el.scrollHeight, top: el.scrollTop };
  }, [messages]);

  const canLoadOlder = older?.hasMore === true && !older.loading;
  const loadOlder = older?.load;
  useEffect(() => {
    const el = topRef.current;
    if (!el || !canLoadOlder || !loadOlder) return;
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) loadOlder();
      },
      { root: listRef.current, rootMargin: "120px 0px 0px 0px" },
    );
    io.observe(el);
    return () => io.disconnect();
  }, [canLoadOlder, loadOlder]);

  async function send() {
    const text = draft.trim();
    if (!text || sending) return;
    if (!onSend) {
      setLocal((m) => [...m, { id: `local-${m.length}`, kind: "me", text, time: "刚刚" }]);
      setDraft("");
      return;
    }
    setSending(true);
    try {
      // 发送期间用户可能已经接着输入了下一条：只清掉刚发出的那段
      if (await onSend(text)) setDraft((d) => (d.trim() === text ? "" : d));
    } finally {
      setSending(false);
    }
  }

  return (
    <section
      className="flex min-w-0 flex-col overflow-hidden rounded-xl border border-border bg-surface"
      aria-label={label}
    >
      {header ? <div className="border-b border-border px-4 py-3">{header}</div> : null}
      <ol
        ref={listRef}
        onScroll={(e) => {
          snapshot.current.top = e.currentTarget.scrollTop;
          snapshot.current.height = e.currentTarget.scrollHeight;
        }}
        className="flex max-h-[28rem] min-h-64 flex-col gap-2 overflow-y-auto px-3 py-4"
      >
        {older ? (
          <li ref={topRef} className="text-center text-xs text-fg-muted">
            {older.loading ? (
              "正在加载更早的消息…"
            ) : older.hasMore ? (
              <button type="button" onClick={older.load} className="hover:text-fg">
                加载更早的消息
              </button>
            ) : messages.length ? (
              "没有更早的消息了"
            ) : null}
          </li>
        ) : null}
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
              data-testid="chat-message"
              className={cn(
                "flex max-w-[80%] min-w-0 flex-col",
                m.kind === "me" ? "items-end self-end" : "items-start",
              )}
            >
              {m.author ? <span className="mb-0.5 text-xs text-fg-muted">{m.author}</span> : null}
              {m.content ?? (
                <span
                  className={cn(
                    "max-w-full rounded-lg px-3 py-2 whitespace-pre-wrap [overflow-wrap:anywhere]",
                    m.kind === "me"
                      ? "rounded-br-sm bg-accent-fill text-accent-fg"
                      : "rounded-bl-sm bg-surface-2",
                  )}
                >
                  {m.text}
                </span>
              )}
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
      {notice ? <div className="border-t border-border px-4 py-2 text-sm">{notice}</div> : null}
      {readOnly || disabledReason ? (
        <p className="border-t border-border px-4 py-3 text-center text-sm text-fg-muted">
          {disabledReason ?? "聊天记录只读，已作为仲裁证据保存"}
        </p>
      ) : (
        <form
          className="flex items-center gap-2 border-t border-border p-2"
          onSubmit={(e) => {
            e.preventDefault();
            void send();
          }}
        >
          <input
            value={draft}
            maxLength={maxLength}
            onChange={(e) => {
              setDraft(e.target.value);
              onDraftChange?.(e.target.value);
            }}
            placeholder={placeholder}
            aria-label="消息内容"
            className="h-10 min-w-0 flex-1 rounded-pill bg-surface-2 px-4 placeholder:text-fg-muted focus-visible:outline-2 focus-visible:outline-accent"
          />
          <button
            type="submit"
            className="inline-flex h-10 shrink-0 items-center gap-1 rounded-pill bg-accent-fill px-4 text-accent-fg disabled:opacity-45"
            disabled={!draft.trim() || sending}
          >
            <SendHorizontal size={18} aria-hidden />
            发送
          </button>
        </form>
      )}
    </section>
  );
}
