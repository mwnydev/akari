"use client";

import { createContext, useContext, useMemo, useState, type ReactNode } from "react";

export interface QuoteTarget {
  floor: number;
  author: string;
}

interface ComposerApi {
  open: boolean;
  quote: QuoteTarget | null;
  /** 打开底部编辑器；传楼层即「引用回复」 */
  openComposer: (quote?: QuoteTarget) => void;
  closeComposer: () => void;
}

const Ctx = createContext<ComposerApi | null>(null);

/** 主题页共享的编辑器状态：楼层里的「引用回复」打开底部编辑器。 */
export function ComposerProvider({ children }: { children: ReactNode }) {
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
  return <Ctx.Provider value={api}>{children}</Ctx.Provider>;
}

export function useComposer(): ComposerApi {
  const api = useContext(Ctx);
  if (!api) throw new Error("useComposer must be used inside ComposerProvider");
  return api;
}
