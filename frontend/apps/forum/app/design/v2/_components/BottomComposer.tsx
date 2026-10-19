"use client";

import { Avatar, Button, cn, Textarea } from "@akari/ui";
import { AtSign, Bold, ImageIcon, Italic, Link2, Quote, X } from "lucide-react";
import { useEffect, useRef, type ReactNode } from "react";
import { useComposer } from "./ComposerContext";

const TOOLS = [
  { label: "加粗", Icon: Bold },
  { label: "斜体", Icon: Italic },
  { label: "链接", Icon: Link2 },
  { label: "图片", Icon: ImageIcon },
  { label: "提及", Icon: AtSign },
] as const;

/** 与 ForumShell 的正文列左对齐（桌面端让出侧栏宽度）。 */
const column = "mx-auto max-w-[90rem] px-3 sm:px-4 lg:pl-[calc(14rem+3rem)]";

/**
 * 贴底的回复编辑器（静态设计稿，正式版换成按需加载的 Tiptap）：通栏面板，顶部分隔线和拖动条，向上展开。
 * 收起时是一条「回复本帖」栏，展开后不遮挡正文滚动（非模态）；点拖动条、「收起」或按 Esc 收起，焦点还给回复栏。
 * `action` 放在收起栏右侧（交易帖的「担保购买」等，仅窄屏）。
 */
export function BottomComposer({ viewer, action }: { viewer: string; action?: ReactNode }) {
  const { open, quote, openComposer, closeComposer } = useComposer();
  const textarea = useRef<HTMLTextAreaElement>(null);
  const bar = useRef<HTMLButtonElement>(null);
  const wasOpen = useRef(false);

  useEffect(() => {
    if (open) textarea.current?.focus();
    else if (wasOpen.current) bar.current?.focus();
    wasOpen.current = open;
  }, [open, quote]);

  return (
    <div className="fixed inset-x-0 bottom-0 z-30">
      {open ? (
        <section
          aria-label="回复编辑器"
          className="animate-composer-in border-t border-border bg-surface shadow-sheet"
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
          <div className={cn(column, "pb-3")}>
            <div className="max-w-3xl">
              <div className="mb-2 flex items-center gap-2">
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
                  <span className="text-sm font-medium">回复本帖</span>
                )}
                <div role="toolbar" aria-label="格式" className="ml-auto hidden gap-0.5 sm:flex">
                  {TOOLS.map(({ label, Icon }) => (
                    <button
                      key={label}
                      type="button"
                      aria-label={label}
                      className="inline-flex size-8 items-center justify-center rounded-md text-fg-muted hover:bg-surface-2 hover:text-fg focus-visible:outline-2 focus-visible:outline-accent"
                    >
                      <Icon size={16} aria-hidden />
                    </button>
                  ))}
                </div>
              </div>
              <Textarea
                ref={textarea}
                aria-label="回复内容"
                placeholder="说点什么…  @ 提及、粘贴图片都可以"
                className="h-32 resize-none sm:h-44"
              />
              <div className="mt-2 flex items-center gap-2">
                <Button size="sm" onClick={closeComposer}>
                  发布回复
                </Button>
                <Button variant="subtle" size="sm" onClick={closeComposer}>
                  收起
                </Button>
                <span className="ml-auto text-xs text-fg-muted">草稿自动保存</span>
              </div>
            </div>
          </div>
        </section>
      ) : (
        <div className="border-t border-border bg-surface">
          <div className={column}>
            <div className="flex max-w-3xl items-center gap-2 py-2">
              <Avatar name={viewer} size={32} />
              <button
                ref={bar}
                type="button"
                onClick={() => openComposer()}
                className={cn(
                  "h-9 min-w-0 flex-1 rounded-md border border-border bg-surface px-3 text-left text-sm text-fg-muted hover:border-fg-muted",
                  "focus-visible:outline-2 focus-visible:outline-accent",
                )}
              >
                回复本帖…
              </button>
              {action}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
