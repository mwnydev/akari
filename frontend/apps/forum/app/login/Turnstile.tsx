"use client";

import { useEffect, useRef } from "react";

// Cloudflare Turnstile（plan 第 11 节「刷帖、广告」）。显式渲染：脚本只在登录页加载。
const SCRIPT_SRC = "https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit";

interface TurnstileApi {
  render(
    el: HTMLElement,
    options: {
      sitekey: string;
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
 * 人机验证组件。令牌只能用一次：`round` 变化时重置，换一个新令牌（每次请求验证码后由父组件加 1）。
 * 令牌过期或出错时回调 null。
 */
export function Turnstile({
  siteKey,
  round,
  onToken,
}: {
  siteKey: string;
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
  }, [siteKey]);

  useEffect(() => {
    if (round > 0 && widget.current) {
      onTokenRef.current(null);
      window.turnstile?.reset(widget.current);
    }
  }, [round]);

  return <div ref={el} className="min-h-[65px]" aria-label="人机验证" />;
}
