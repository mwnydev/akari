import { ApiError, createApi } from "@akari/api-client";

/** 浏览器端客户端：同源 /api/v1，会话 Cookie 由浏览器自动带上。 */
export const api = createApi();

/** 等待请求：非 2xx（含网络错误）抛 ApiError，给 TanStack Query 的 onError 用。204 返回 undefined。 */
export async function unwrap<T>(
  request: Promise<{ data?: T; error?: unknown; response: Response }>,
): Promise<T> {
  let res: Awaited<typeof request>;
  try {
    res = await request;
  } catch {
    throw ApiError.from(null, 0);
  }
  if (!res.response.ok) throw ApiError.from(res.error, res.response.status);
  return res.data as T;
}
