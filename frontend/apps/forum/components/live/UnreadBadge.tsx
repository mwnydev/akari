"use client";

import { useEffect, useState } from "react";
import { badgeText } from "../../lib/badge";
import { countDot } from "../forum/styles";

/**
 * 顶栏通知图标上的未读数。数字在浏览器里取（页面本身不含按用户变化的计数），
 * 实时连接的代码在挂载后才加载，不占首屏 JS。
 */
export function UnreadBadge() {
  const [count, setCount] = useState(0);
  useEffect(() => {
    let stop: (() => void) | undefined;
    let cancelled = false;
    void import("../../lib/realtime/unread").then(({ watchUnread }) => {
      if (!cancelled) stop = watchUnread(setCount);
    });
    return () => {
      cancelled = true;
      stop?.();
    };
  }, []);
  const badge = badgeText(count);
  return badge ? (
    <>
      <span aria-hidden className={countDot} data-testid="unread-badge">
        {badge}
      </span>
      <span className="sr-only">，{badge} 条未读</span>
    </>
  ) : null;
}
