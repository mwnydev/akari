"use client";

import { createContext, useContext, useMemo, useRef, useState, type ReactNode } from "react";
import { PostCaptchaProvider } from "../captcha/PostCaptcha";

/** 「引用」：被引用的楼层，正文摘录插进编辑器开头的引用块。 */
export interface QuoteTarget {
  floor: number;
  author: string;
  /** 被引用楼层的地址 */
  href: string;
  text: string;
}

/**
 * 编辑器从哪里打开（弹窗标题「回复 #2 张三」「引用 #2 张三」）：
 * - `reply`：楼层上的「回复」，`mention` 不为空时编辑器开头先 @ 这层楼的作者（对方会收到提及通知）；
 * - `quote`：楼层上的「引用」，开头插入引用块。
 */
export type ComposeContext =
  | { kind: "reply"; floor: number; author: string; mention: { id: number; label: string } | null }
  | ({ kind: "quote" } & QuoteTarget);

interface ComposerApi {
  open: boolean;
  context: ComposeContext | null;
  /** 打开编辑器弹窗；不传 = 回复本帖 / 发新帖 */
  openComposer: (context?: ComposeContext) => void;
  closeComposer: () => void;
  /** 关闭后把焦点还给打开它的按钮 */
  restoreFocus: () => void;
}

const Ctx = createContext<ComposerApi | null>(null);

/**
 * 页面共享的编辑器状态：楼层里的「回复」「引用」、回复栏、列表顶部的「发新帖」都打开同一个居中弹窗。
 * `captcha`：访问者发帖要先过人机验证（needsPostCaptcha），编辑器与楼层评论框据此显示 Turnstile。
 */
export function ComposerProvider({
  captcha = false,
  children,
}: {
  captcha?: boolean;
  children: ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const [context, setContext] = useState<ComposeContext | null>(null);
  const opener = useRef<HTMLElement | null>(null);
  const api = useMemo<ComposerApi>(
    () => ({
      open,
      context,
      openComposer: (c) => {
        if (!open) {
          opener.current =
            document.activeElement instanceof HTMLElement ? document.activeElement : null;
        }
        setContext(c ?? null);
        setOpen(true);
      },
      closeComposer: () => setOpen(false),
      restoreFocus: () => {
        if (opener.current?.isConnected) opener.current.focus();
        opener.current = null;
      },
    }),
    [open, context],
  );
  return (
    <Ctx.Provider value={api}>
      <PostCaptchaProvider required={captcha}>{children}</PostCaptchaProvider>
    </Ctx.Provider>
  );
}

/** 没有 ComposerProvider（不能回复的页面）时返回 null，调用方隐藏相应按钮。 */
export function useComposer(): ComposerApi | null {
  return useContext(Ctx);
}
