"use client";

import { Avatar, cn, LoadingState } from "@akari/ui";
import dynamic from "next/dynamic";
import { useState, type ReactNode } from "react";
import type { UserCardView } from "../../lib/display";
import { useAfterSubmit } from "../composer/useAfterSubmit";
import { useComposer } from "./ComposerContext";

// 编辑器弹窗（Tiptap + Radix 弹窗）只在打开时下载（plan 第 12 节首屏 JS 预算）；指针移上回复栏时提前预取。
const loadDialog = () => import("../composer/ComposerDialog");
const ComposerDialog = dynamic(loadDialog, {
  ssr: false,
  loading: () => (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/45">
      <div className="w-[min(48rem,calc(100vw-2rem))] rounded-lg bg-surface p-4 shadow-sheet">
        <LoadingState rows={3} label="正在加载编辑器" />
      </div>
    </div>
  ),
});

/** 楼层上的「回复」「引用」、「发新帖」按钮在指针移入时预取编辑器。 */
export function prefetchComposer(): void {
  void loadDialog();
}

/** 与 ForumShell 的正文列左对齐（桌面端让出侧栏宽度）。 */
const column = "mx-auto max-w-[90rem] px-3 sm:px-4 lg:pl-[calc(14rem+3rem)]";

export type BottomComposerMode =
  /** 主题页：贴底一条「回复本帖」栏 */
  | { kind: "reply"; topicId: number }
  /** 列表页：不占位，由「发新帖」按钮打开；`boards` 多于一个时先选板块 */
  | { kind: "topic"; boards: { slug: string; name: string }[] };

/**
 * 主题页贴底的回复栏，以及页面共用的编辑器弹窗（居中，移动端全屏）：回复栏、楼层的「回复」「引用」、
 * 「发新帖」都打开它。回复模式下 `blocked` 替换回复栏的按钮（游客的登录提示、锁帖说明）；
 * `action` 放在回复栏右侧（拍卖帖的「出价」）。
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
  const boards = mode.kind === "topic" ? mode.boards : [];
  const [board, setBoard] = useState(boards[0]?.slug ?? "");

  if (!composer) return null;
  const { open, context, closeComposer, openComposer, restoreFocus } = composer;
  const target =
    mode.kind === "reply"
      ? ({ kind: "reply", topicId: mode.topicId } as const)
      : board
        ? ({ kind: "topic", forumSlug: board } as const)
        : null;

  return (
    <>
      {mode.kind === "reply" ? (
        <>
          {/* 回复栏是 fixed 的：正文末尾留出同样高度，最后一层不被挡住，也不会在栏出现时跳版 */}
          <div aria-hidden className="h-14" />
          <div className="fixed inset-x-0 bottom-0 z-30 border-t border-border bg-surface">
            <div className={column}>
              <div className="flex max-w-3xl items-center gap-2 py-2">
                {viewer ? (
                  <Avatar name={viewer.name} src={viewer.avatarUrl ?? undefined} size={32} />
                ) : null}
                {blocked ?? (
                  <button
                    type="button"
                    aria-haspopup="dialog"
                    onClick={() => openComposer()}
                    onPointerEnter={prefetchComposer}
                    onFocus={prefetchComposer}
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
        </>
      ) : null}
      {open && target ? (
        <ComposerDialog
          target={target}
          context={mode.kind === "reply" ? context : null}
          boards={boards}
          onBoardChange={setBoard}
          mentionable={mentionable}
          onClose={closeComposer}
          onClosed={restoreFocus}
          onDone={(href) => {
            closeComposer();
            afterSubmit(href);
          }}
        />
      ) : null}
    </>
  );
}
