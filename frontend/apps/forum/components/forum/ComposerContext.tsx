"use client";

import { createContext, useContext, useMemo, useState, type ReactNode } from "react";
import { PostCaptchaProvider } from "../captcha/PostCaptcha";

/** 「引用回复」：被引用的楼层，正文摘录插进编辑器开头的引用块。 */
export interface QuoteTarget {
  floor: number;
  author: string;
  /** 被引用楼层的地址 */
  href: string;
  text: string;
}

interface ComposerApi {
  open: boolean;
  quote: QuoteTarget | null;
  /** 打开底部编辑器；传楼层即「引用回复」 */
  openComposer: (quote?: QuoteTarget) => void;
  closeComposer: () => void;
}

const Ctx = createContext<ComposerApi | null>(null);

/**
 * 页面共享的底部编辑器状态：楼层里的「引用回复」、列表顶部的「发新帖」都打开同一个底部编辑器。
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
  const [quote, setQuote] = useState<QuoteTarget | null>(null);
  const api = useMemo<ComposerApi>(
    () => ({
      open,
      quote,
      openComposer: (q) => {
        setQuote(q ?? null);
        setOpen(true);
      },
      closeComposer: () => setOpen(false),
    }),
    [open, quote],
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
