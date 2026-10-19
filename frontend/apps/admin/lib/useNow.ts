"use client";

import { useEffect, useState } from "react";

/** 定时更新的当前时间（Unix 秒），会话倒计时用。 */
export function useNowSecs(intervalMs = 1000): number {
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));
  useEffect(() => {
    const t = setInterval(() => setNow(Math.floor(Date.now() / 1000)), intervalMs);
    return () => clearInterval(t);
  }, [intervalMs]);
  return now;
}

export function nowSecs(): number {
  return Math.floor(Date.now() / 1000);
}
