"use client";

import { createContext, useContext, useState, type ReactNode } from "react";
import { TURNSTILE_SITE_KEY } from "../../lib/public-config";
import { Turnstile } from "./Turnstile";

// 注册 24 小时内发主题、回帖、评论要先过人机验证（后端 `posting::check_new_user_captcha`）。
// 页面在服务端按 /me 的 `new_user` 决定要不要（lib/public-config 的 needsPostCaptcha），表单里用 usePostCaptcha
// 取组件与令牌。

const Required = createContext(false);

export function PostCaptchaProvider({
  required,
  children,
}: {
  required: boolean;
  children: ReactNode;
}) {
  return <Required.Provider value={required}>{children}</Required.Provider>;
}

/**
 * 发帖表单里的人机验证：`widget` 放进表单（不需要时为 null），`token` 随请求提交，`missing` 时禁用提交按钮；
 * 令牌只能用一次，每次提交后（无论成败）调 `renew` 换新的。
 */
export function usePostCaptcha() {
  const required = useContext(Required);
  const [token, setToken] = useState<string | null>(null);
  const [round, setRound] = useState(0);
  return {
    widget: required ? (
      <Turnstile siteKey={TURNSTILE_SITE_KEY} action="post" round={round} onToken={setToken} />
    ) : null,
    token,
    missing: required && token === null,
    renew: () => {
      setToken(null);
      setRound((n) => n + 1);
    },
  };
}
