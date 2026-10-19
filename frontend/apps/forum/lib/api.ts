import "server-only";
import { ApiError, createApi, type ApiClient } from "@akari/api-client";
import { cookies } from "next/headers";
import { cache } from "react";
import { API_ORIGIN, SESSION_COOKIE } from "./config";

/**
 * 本次请求的服务端 API 客户端。登录用户转发会话 Cookie（proxy.ts 已验签，页面不进缓存）；
 * 游客不带 Cookie，后端返回与访问者无关的内容，整页可由 Cloudflare 缓存 60 秒（plan 第 8 节）。
 */
export const serverApi = cache(async (): Promise<ApiClient> => {
  const sid = (await cookies()).get(SESSION_COOKIE)?.value;
  return createApi({
    baseUrl: API_ORIGIN,
    cache: "no-store",
    headers: sid ? { cookie: `${SESSION_COOKIE}=${sid}` } : {},
  });
});

/** 是否带着会话渲染（决定要不要请求只对登录用户有意义的接口）。 */
export async function hasSession(): Promise<boolean> {
  return (await cookies()).has(SESSION_COOKIE);
}

/**
 * 等待一次 openapi-fetch 请求：成功返回数据，失败（含网络错误）返回 ApiError，由页面决定 404 还是出错状态。
 * 不抛异常：生产环境里 Server Component 抛出的错误信息会被隐去，用户就看不到中文提示了。
 */
export async function load<T>(
  request: Promise<{ data?: T; error?: unknown; response: Response }>,
): Promise<T | ApiError> {
  try {
    const { data, error, response } = await request;
    return data !== undefined ? data : ApiError.from(error, response.status);
  } catch {
    return ApiError.from(null, 0);
  }
}
