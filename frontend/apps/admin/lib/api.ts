import { ApiError, createApi, type Schemas } from "@akari/api-client";
import { API_ORIGIN } from "./config";

export type AdminSessionView = Schemas["AdminSessionView"];

/** 后台接口客户端：跨源到论坛主机，带上论坛会话与后台会话 Cookie（都是 host-only，跨子域名仍是同站请求）。 */
export const api = createApi({ baseUrl: API_ORIGIN, credentials: "include" });

type Listener = (status: number, err: ApiError | null) => void;
const listeners = new Set<Listener>();

/** 订阅每个后台请求的结果（会话监控用：成功即顺延闲置期限，401 即会话已失效）。 */
export function onApiResult(fn: Listener): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

/** 等待请求：非 2xx（含网络错误）抛 ApiError，给 TanStack Query 的 onError 用。204 返回 undefined。 */
export async function unwrap<T>(
  request: Promise<{ data?: T; error?: unknown; response: Response }>,
): Promise<T> {
  let res: Awaited<typeof request>;
  try {
    res = await request;
  } catch {
    const err = ApiError.from(null, 0);
    listeners.forEach((l) => l(0, err));
    throw err;
  }
  const err = res.response.ok ? null : ApiError.from(res.error, res.response.status);
  listeners.forEach((l) => l(res.response.status, err));
  if (err) throw err;
  return res.data as T;
}
