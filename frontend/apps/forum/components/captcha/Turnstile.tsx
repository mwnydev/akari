"use client";

import { useEffect, useRef } from "react";

// Cloudflare Turnstile（plan 第 11 节「刷帖、广告」）。显式渲染：第三方脚本只在用到的表单（登录、新用户发帖）
// 挂载时才插入页面，不进任何页面的首屏 JS。
const SCRIPT_SRC = "https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit";

interface TurnstileApi {
  render(
    el: HTMLElement,
    options: {
      sitekey: string;
      action: string;
      language?: string;
      callback: (token: string) => void;
      "expired-callback": () => void;
      "error-callback": () => void;
    },
  ): string;
  reset(widgetId: string): void;
  remove(widgetId: string): void;
}

declare global {
  interface Window {
    turnstile?: TurnstileApi;
  }
}

let scriptPromise: Promise<TurnstileApi> | undefined;

function loadTurnstile(): Promise<TurnstileApi> {
  scriptPromise ??= new Promise((resolve, reject) => {
    const s = document.createElement("script");
    s.src = SCRIPT_SRC;
    s.async = true;
    s.onload = () =>
      window.turnstile ? resolve(window.turnstile) : reject(new Error("turnstile"));
    s.onerror = () => {
      scriptPromise = undefined;
      reject(new Error("turnstile script failed"));
    };
    document.head.appendChild(s);
  });
  return scriptPromise;
}

/**
 * 人机验证组件。令牌只能用一次：`round` 变化时重置，换一个新令牌（每次提交后由父组件加 1）。
 * 令牌过期或出错时回调 null。`action` 与服务端核对（`login` / `post`），一个表单的令牌不能拿去别的表单用。
 */
export function Turnstile({
  siteKey,
  action,
  round,
  onToken,
}: {
  siteKey: string;
  action: "login" | "post";
  round: number;
  onToken: (token: string | null) => void;
}) {
  const el = useRef<HTMLDivElement>(null);
  const widget = useRef<string | null>(null);
  const onTokenRef = useRef(onToken);
  useEffect(() => {
    onTokenRef.current = onToken;
  });

  useEffect(() => {
    let cancelled = false;
    loadTurnstile()
      .then((t) => {
        if (cancelled || !el.current) return;
        widget.current = t.render(el.current, {
          sitekey: siteKey,
          action,
          language: "zh-cn",
          callback: (token) => onTokenRef.current(token),
          "expired-callback": () => onTokenRef.current(null),
          "error-callback": () => onTokenRef.current(null),
        });
      })
      .catch(() => onTokenRef.current(null));
    return () => {
      cancelled = true;
      if (widget.current) window.turnstile?.remove(widget.current);
      widget.current = null;
    };
  }, [siteKey, action]);

  useEffect(() => {
    if (round > 0 && widget.current) {
      onTokenRef.current(null);
      window.turnstile?.reset(widget.current);
    }
  }, [round]);

  return <div ref={el} className="min-h-[65px]" aria-label="人机验证" />;
}
