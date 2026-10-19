"use client";

import * as DialogPrimitive from "@radix-ui/react-dialog";
import { X } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "../cn";

export const Dialog = DialogPrimitive.Root;
export const DialogTrigger = DialogPrimitive.Trigger;
export const DialogClose = DialogPrimitive.Close;

/** 玻璃弹窗：移动端贴底的面板，桌面端居中。 */
export function DialogContent({
  title,
  description,
  children,
  className,
}: {
  title: string;
  description?: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <DialogPrimitive.Portal>
      <DialogPrimitive.Overlay className="fixed inset-0 z-40 animate-fade-in bg-black/45 backdrop-blur-(--blur-overlay)" />
      <DialogPrimitive.Content
        className={cn(
          "glass-strong fixed top-1/2 left-1/2 z-50 w-[calc(100vw-24px)] max-w-md -translate-x-1/2 -translate-y-1/2",
          "animate-sheet-in rounded-xl p-5 shadow-sheet focus:outline-none",
          className,
        )}
      >
        <div className="mb-3 flex items-start justify-between gap-3">
          <div>
            <DialogPrimitive.Title className="text-lg font-semibold">{title}</DialogPrimitive.Title>
            {description ? (
              <DialogPrimitive.Description className="text-sm text-fg-muted">
                {description}
              </DialogPrimitive.Description>
            ) : null}
          </div>
          <DialogPrimitive.Close
            className="-mt-1 -mr-1 inline-flex size-8 items-center justify-center rounded-full text-fg-muted hover:bg-surface-2"
            aria-label="关闭"
          >
            <X size={18} aria-hidden />
          </DialogPrimitive.Close>
        </div>
        {children}
      </DialogPrimitive.Content>
    </DialogPrimitive.Portal>
  );
}

/** 左侧抽屉（移动端的板块导航等）：实底面板，同样是模态对话框，焦点锁在抽屉里，Esc 或点遮罩关闭。 */
export function DrawerContent({
  title,
  children,
  className,
}: {
  title: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <DialogPrimitive.Portal>
      <DialogPrimitive.Overlay className="fixed inset-0 z-40 animate-fade-in bg-black/45 backdrop-blur-(--blur-overlay)" />
      <DialogPrimitive.Content
        aria-describedby={undefined}
        className={cn(
          "fixed inset-y-0 left-0 z-50 flex w-[min(20rem,85vw)] flex-col border-r border-border bg-surface",
          "animate-drawer-in overflow-y-auto p-4 shadow-sheet focus:outline-none",
          className,
        )}
      >
        <div className="mb-2 flex items-center justify-between gap-3">
          <DialogPrimitive.Title className="font-semibold">{title}</DialogPrimitive.Title>
          <DialogPrimitive.Close
            className="inline-flex size-8 items-center justify-center rounded-full text-fg-muted hover:bg-surface-2 focus-visible:outline-2 focus-visible:outline-accent"
            aria-label="关闭"
          >
            <X size={18} aria-hidden />
          </DialogPrimitive.Close>
        </div>
        {children}
      </DialogPrimitive.Content>
    </DialogPrimitive.Portal>
  );
}
