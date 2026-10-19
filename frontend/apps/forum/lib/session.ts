import "server-only";
import { createApi, type Me } from "@akari/api-client";
import { cookies } from "next/headers";
import { API_ORIGIN, SESSION_COOKIE } from "./config";

/**
 * 当前登录用户；游客或会话失效返回 null。
 * 签名不对的 Cookie 已被 proxy.ts 剥掉，这里看到的 sid 一定是我们签发的（是否过期由后端判断）。
 */
export async function getMe(): Promise<Me | null> {
  const sid = (await cookies()).get(SESSION_COOKIE)?.value;
  if (!sid) return null;
  const api = createApi({ baseUrl: API_ORIGIN });
  try {
    const { data } = await api.GET("/api/v1/me", {
      headers: { cookie: `${SESSION_COOKIE}=${sid}` },
      cache: "no-store",
    });
    return data ?? null;
  } catch {
    // API 不可用时按游客渲染，页面照常可读
    return null;
  }
}
