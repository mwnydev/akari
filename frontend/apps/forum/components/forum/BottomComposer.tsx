"use client";

import { Avatar, cn, LoadingState } from "@akari/ui";
import { Quote, X } from "lucide-react";
import dynamic from "next/dynamic";
import { useEffect, useRef, useState, type ReactNode } from "react";
import type { UserCardView } from "../../lib/display";
import type { ComposeTarget } from "../composer/target";
import { useAfterSubmit } from "../composer/useAfterSubmit";
import { useComposer } from "./ComposerContext";
import { selectClass } from "./styles";

// Tiptap 只在展开编辑器时下载（plan 第 12 节首屏 JS 预算）；指针移上回复栏时提前预取。
const loadComposer = () => import("../composer/Composer");
const Composer = dynamic(loadComposer, {
  ssr: false,
  loading: () => <LoadingState rows={2} label="正在加载编辑器" />,
});

/** 与 ForumShell 的正文列左对齐（桌面端让出侧栏宽度）。 */
const column = "mx-auto max-w-[90rem] px-3 sm:px-4 lg:pl-[calc(14rem+3rem)]";

export type BottomComposerMode =
  /** 主题页：收起时是一条「回复本帖」栏 */
  | { kind: "reply"; topicId: number }
  /** 列表页：收起时不占位，由「发新帖」按钮展开；`boards` 多于一个时先选板块 */
  | { kind: "topic"; boards: { slug: string; name: string }[] };

/**
 * 贴底的编辑器：通栏面板，顶部分隔线和拖动条，向上展开；展开后不遮挡正文滚动（非模态）。
 * 点拖动条、「取消」或按 Esc 收起，焦点还给回复栏。编辑器本身（Tiptap）按需加载。
 * 回复模式下 `blocked` 替换回复栏的按钮（游客的登录提示、锁帖说明）；`action` 放在回复栏右侧（交易帖的「担保购买」）。
 */
export function BottomComposer({
  mode,
  viewer,
  mentionable = [],
  blocked,
  action,
}: {
  mode: BottomComposerMode;
  viewer?: { name: string; avatarUrl?: string | null | undefined } | null;
  mentionable?: UserCardView[];
  blocked?: ReactNode;
  action?: ReactNode;
}) {
  const composer = useComposer();
  const afterSubmit = useAfterSubmit();
  const bar = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLElement>(null);
  const wasOpen = useRef(false);
  const boards = mode.kind === "topic" ? mode.boards : [];
  const [board, setBoard] = useState(boards[0]?.slug ?? "");
  const open = composer?.open ?? false;
  const quote = composer?.quote ?? null;

  useEffect(() => {
    if (open) panel.current?.focus();
    else if (wasOpen.current) bar.current?.focus();
    wasOpen.current = open;
  }, [open, quote]);

  if (!composer) return null;
  const { closeComposer, openComposer } = composer;

  const target: Exclude<ComposeTarget, { kind: "edit" }> | null =
    mode.kind === "reply"
      ? { kind: "reply", topicId: mode.topicId }
      : board
        ? { kind: "topic", forumSlug: board }
        : null;

  const collapsed =
    mode.kind === "reply" ? (
      <div className="border-t border-border bg-surface">
        <div className={column}>
          <div className="flex max-w-3xl items-center gap-2 py-2">
            {viewer ? (
              <Avatar name={viewer.name} src={viewer.avatarUrl ?? undefined} size={32} />
            ) : null}
            {blocked ?? (
              <button
                ref={bar}
                type="button"
                onClick={() => openComposer()}
                onPointerEnter={() => void loadComposer()}
                onFocus={() => void loadComposer()}
                className={cn(
                  "h-9 min-w-0 flex-1 rounded-md border border-border bg-surface px-3 text-left text-sm text-fg-muted hover:border-fg-muted",
                  "focus-visible:outline-2 focus-visible:outline-accent",
                )}
              >
                回复本帖…
              </button>
            )}
            {action}
          </div>
        </div>
      </div>
    ) : null;

  return (
    <>
      {/* 回复栏是 fixed 的：正文末尾留出同样高度，最后一层不被挡住，也不会在栏出现时跳版 */}
      {mode.kind === "reply" ? <div aria-hidden className="h-14" /> : null}
      <div className="fixed inset-x-0 bottom-0 z-30">
        {open && target ? (
          <section
            ref={panel}
            tabIndex={-1}
            aria-label={mode.kind === "reply" ? "回复编辑器" : "发新帖"}
            className="animate-composer-in border-t border-border bg-surface shadow-sheet outline-none"
            onKeyDown={(e) => {
              if (e.key === "Escape") closeComposer();
            }}
          >
            <button
              type="button"
              aria-label="收起编辑器"
              onClick={closeComposer}
              className="flex h-4 w-full cursor-pointer items-center justify-center hover:bg-surface-2 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent"
            >
              <span aria-hidden className="h-1 w-10 rounded-full bg-fg-muted/40" />
            </button>
            <div className={cn(column, "max-h-[80dvh] overflow-y-auto pb-3")}>
              <div className="max-w-3xl">
                <div className="mb-2 flex flex-wrap items-center gap-2">
                  {quote ? (
                    <span className="inline-flex min-w-0 items-center gap-1 rounded-sm bg-accent-soft px-2 py-0.5 text-sm text-accent">
                      <Quote size={14} aria-hidden />
                      <span className="truncate">
                        引用 #{quote.floor} {quote.author}
                      </span>
                      <button
                        type="button"
                        aria-label="取消引用"
                        className="-mr-1 rounded-sm p-0.5 hover:bg-accent-soft focus-visible:outline-2 focus-visible:outline-accent"
                        onClick={() => openComposer()}
                      >
                        <X size={14} aria-hidden />
                      </button>
                    </span>
                  ) : (
                    <span className="text-sm font-medium">
                      {mode.kind === "reply" ? "回复本帖" : "发新帖"}
                    </span>
                  )}
                  {boards.length > 1 ? (
                    <label className="ml-auto flex items-center gap-2 text-sm">
                      <span className="shrink-0 text-fg-muted">发到</span>
                      <select
                        value={board}
                        onChange={(e) => setBoard(e.target.value)}
                        className={cn(selectClass, "h-8 w-auto")}
                      >
                        {boards.map((b) => (
                          <option key={b.slug} value={b.slug}>
                            {b.name}
                          </option>
                        ))}
                      </select>
                    </label>
                  ) : null}
                </div>
                <div className="[&_.tiptap]:max-h-[40dvh] [&_.tiptap]:min-h-28 [&_.tiptap]:overflow-y-auto">
                  <Composer
                    // 换板块或换引用时重新开一份（草稿按板块分开存）
                    key={`${target.kind === "topic" ? target.forumSlug : "reply"}:${quote?.floor ?? ""}`}
                    target={target}
                    mentionable={mentionable}
                    quote={quote}
                    autoFocus
                    onCancel={closeComposer}
                    onDone={(href) => {
                      closeComposer();
                      afterSubmit(href);
                    }}
                  />
                </div>
              </div>
            </div>
          </section>
        ) : (
          collapsed
        )}
      </div>
    </>
  );
}
