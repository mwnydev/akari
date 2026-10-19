"use client";

import { useEffect, useState } from "react";

/** 定时更新的当前时间（毫秒），期限与冷却倒计时用（plan 第 14 节）。 */
export function useNow(intervalMs = 30_000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(t);
  }, [intervalMs]);
  return now;
}
