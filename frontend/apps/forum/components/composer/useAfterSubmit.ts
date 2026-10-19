"use client";

import { useRouter } from "next/navigation";
import { useEffect, useRef, useTransition } from "react";

/**
 * 提交成功后的导航：去别的页面就跳转；还在同一页（如回复落在当前页）就刷新服务端组件，
 * 刷新完成后滚到新楼层（只改 hash 的跳转不会重新取数据）。`null` 表示原地刷新。
 */
export function useAfterSubmit(): (href: string | null) => void {
  const router = useRouter();
  const [refreshing, startTransition] = useTransition();
  const scrollTo = useRef<string | null>(null);

  useEffect(() => {
    const hash = scrollTo.current;
    if (refreshing || !hash) return;
    scrollTo.current = null;
    history.replaceState(history.state, "", hash);
    document.getElementById(hash.slice(1))?.scrollIntoView({ block: "start" });
  }, [refreshing]);

  return (href) => {
    if (href !== null) {
      const url = new URL(href, location.href);
      if (url.pathname + url.search !== location.pathname + location.search) {
        router.push(href);
        return;
      }
      scrollTo.current = url.hash || null;
    }
    startTransition(() => router.refresh());
  };
}
