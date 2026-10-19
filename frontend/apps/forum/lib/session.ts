import "server-only";
import { ApiError, type Me } from "@akari/api-client";
import { redirect } from "next/navigation";
import { cache } from "react";
import { hasSession, load, serverApi } from "./api";
import { loginHref } from "./links";

/**
 * 当前登录用户；游客或会话失效返回 null。
 * 签名不对的 Cookie 已被 proxy.ts 剥掉，这里看到的 sid 一定是我们签发的（是否过期由后端判断）。
 * API 不可用时也按游客渲染，页面照常可读。
 */
export const getMe = cache(async (): Promise<Me | null> => {
  if (!(await hasSession())) return null;
  const me = await load((await serverApi()).GET("/api/v1/me"));
  return me instanceof ApiError ? null : me;
});

/** 只给登录用户的页面：游客跳到登录页，登录后回到 `next`。 */
export async function requireMe(next: string): Promise<Me> {
  const me = await getMe();
  if (!me) redirect(loginHref(next));
  return me;
}
