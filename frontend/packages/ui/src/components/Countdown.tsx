"use client";

import { useEffect, useState } from "react";
import { cn } from "../cn";

/** 剩余秒数 →「2 天 3 小时」「2:14:05」「14:05」；一天以上不显示秒，减少跳动。 */
export function formatRemaining(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  const d = Math.floor(s / 86_400);
  const h = Math.floor((s % 86_400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  const pad = (n: number) => String(n).padStart(2, "0");
  if (d > 0) return `${d} 天 ${h} 小时`;
  if (h > 0) return `${h}:${pad(m)}:${pad(s % 60)}`;
  return `${pad(m)}:${pad(s % 60)}`;
}

/** 每秒减 1 的倒计时秒数（重发验证码）；`set(n)` 重新开始。 */
export function useCountdown() {
  const [left, setLeft] = useState(0);
  useEffect(() => {
    if (left <= 0) return;
    const t = setTimeout(() => setLeft((n) => n - 1), 1000);
    return () => clearTimeout(t);
  }, [left]);
  return [left, setLeft] as const;
}

/**
 * 倒计时：服务端按 `seconds` 渲染首帧，挂载后每秒递减；首帧与水合结果一致，等宽数字不跳版。
 * 不设 aria-live，屏幕阅读器不会每秒播报。
 */
export function Countdown({
  seconds,
  ended = "已结束",
  className,
}: {
  seconds: number;
  /** 归零后显示的文字 */
  ended?: string;
  className?: string;
}) {
  const [left, setLeft] = useState(seconds);
  useEffect(() => {
    const start = Date.now();
    const id = setInterval(() => {
      const next = seconds - Math.floor((Date.now() - start) / 1000);
      setLeft(next);
      if (next <= 0) clearInterval(id);
    }, 1000);
    return () => clearInterval(id);
  }, [seconds]);
  return (
    <span className={cn("amount whitespace-nowrap", className)}>
      {left > 0 ? formatRemaining(left) : ended}
    </span>
  );
}
