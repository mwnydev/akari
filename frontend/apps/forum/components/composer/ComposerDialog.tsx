"use client";

import { Button, cn } from "@akari/ui";
import { Dialog, PanelDialogContent } from "@akari/ui/dialog";
import { useRef, useState } from "react";
import type { UserCardView } from "../../lib/display";
import type { ComposeContext } from "../forum/ComposerContext";
import { selectClass } from "../forum/styles";
import Composer from "./Composer";
import type { ComposeTarget } from "./target";

export interface ComposerDialogProps {
  target: Exclude<ComposeTarget, { kind: "edit" }>;
  context: ComposeContext | null;
  /** 发新帖：可选的板块（多于一个时标题行里先选板块） */
  boards: { slug: string; name: string }[];
  onBoardChange: (slug: string) => void;
  mentionable: UserCardView[];
  onClose: () => void;
  onDone: (href: string | null) => void;
  /** 关闭后焦点回到打开弹窗的按钮 */
  onClosed: () => void;
}

/** 弹窗标题：回复 #2 张三 / 引用 #2 张三 / 回复本帖 / 发新帖。 */
function dialogTitle(target: ComposerDialogProps["target"], c: ComposeContext | null) {
  if (target.kind === "topic") return "发新帖";
  if (!c) return "回复本帖";
  return `${c.kind === "quote" ? "引用" : "回复"} #${c.floor} ${c.author}`;
}

/**
 * 居中的编辑器弹窗（移动端全屏）：标题写明回复谁，× / Esc / 点遮罩关闭；写了内容还没发出去时先确认
 * （内容自动存为本机草稿，下次打开同一处会恢复）。按需加载：Tiptap 与 Radix 弹窗都在这个分包里。
 */
export default function ComposerDialog({
  target,
  context,
  boards,
  onBoardChange,
  mentionable,
  onClose,
  onDone,
  onClosed,
}: ComposerDialogProps) {
  // 编辑器同步报告（「放弃草稿」先报告没改过、再请求关闭），用 ref 读最新值
  const dirty = useRef(false);
  const [confirming, setConfirming] = useState(false);

  const requestClose = () => {
    if (dirty.current) setConfirming(true);
    else onClose();
  };

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) requestClose();
      }}
    >
      <PanelDialogContent
        title={dialogTitle(target, context)}
        toolbar={
          target.kind === "topic" && boards.length > 1 ? (
            <label className="flex items-center gap-2 text-sm">
              <span className="shrink-0 text-fg-muted">发到</span>
              <select
                value={target.forumSlug}
                onChange={(e) => onBoardChange(e.target.value)}
                className={cn(selectClass, "h-8 w-auto")}
              >
                {boards.map((b) => (
                  <option key={b.slug} value={b.slug}>
                    {b.name}
                  </option>
                ))}
              </select>
            </label>
          ) : null
        }
        onCloseAutoFocus={(e) => {
          e.preventDefault();
          onClosed();
        }}
      >
        {confirming ? (
          <div
            role="alertdialog"
            aria-label="关闭编辑器"
            className="mb-3 flex flex-wrap items-center gap-2 rounded-md bg-warning-soft px-3 py-2 text-sm"
          >
            <p className="min-w-0 flex-1">
              关闭编辑器？没发表的内容已自动存为本机草稿，下次打开会恢复。
            </p>
            <Button autoFocus size="sm" variant="secondary" onClick={() => setConfirming(false)}>
              继续编辑
            </Button>
            <Button size="sm" variant="ghost" onClick={onClose}>
              仍然关闭
            </Button>
          </div>
        ) : null}
        <div className="[&_.tiptap]:max-h-[50dvh] [&_.tiptap]:min-h-40 [&_.tiptap]:overflow-y-auto">
          <Composer
            // 换板块或换楼层时重新开一份（草稿按板块 / 主题分开存）
            key={`${target.kind === "topic" ? target.forumSlug : "reply"}:${context?.kind ?? ""}:${context?.floor ?? ""}`}
            target={target}
            mentionable={mentionable}
            context={context}
            autoFocus
            onDirtyChange={(d) => {
              dirty.current = d;
            }}
            onCancel={requestClose}
            onDone={onDone}
          />
        </div>
      </PanelDialogContent>
    </Dialog>
  );
}
