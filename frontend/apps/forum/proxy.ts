// 会话感知缓存（plan 第 8 节「游客页面」）：
// - 带有效签名会话 Cookie → 按登录用户渲染，禁止任何缓存；
// - 没有或伪造的 Cookie → 剥掉后按游客渲染，允许 Cloudflare 缓存 60 秒（后端另有一层 Valkey 游客响应缓存，见 forum-api 的 guest 模块）。
// Cloudflare 侧须配缓存规则：HTML 可缓存（遵循 Cloudflare-CDN-Cache-Control），带 `sid` Cookie 的请求绕过缓存
// （docs/perf/2026-10-load-test.md「上线配置」）；否则登录用户会拿到边缘缓存的游客页。
// 伪造的 Cookie 因此打不穿缓存层去触发按用户渲染（plan 第 11 节）。
import { type NextRequest, NextResponse } from "next/server";
import { SESSION_COOKIE } from "./lib/config";
import { GUEST_EDGE_CACHE } from "./lib/edge-cache";
import { importSigningKey, verifySignedValue } from "./lib/signed-cookie";

let keyPromise: Promise<CryptoKey | null> | undefined;

function signingKey(): Promise<CryptoKey | null> {
  keyPromise ??= (async () => {
    const key = process.env.COOKIE_SIGNING_KEY;
    if (!key) {
      console.error("COOKIE_SIGNING_KEY is not set: every request is rendered as guest");
      return null;
    }
    return importSigningKey(key);
  })();
  return keyPromise;
}

async function hasValidSession(req: NextRequest): Promise<boolean> {
  const value = req.cookies.get(SESSION_COOKIE)?.value;
  if (!value) return false;
  const key = await signingKey();
  return key !== null && (await verifySignedValue(key, value)) !== null;
}

export async function proxy(req: NextRequest) {
  if (await hasValidSession(req)) {
    const res = NextResponse.next();
    res.headers.set("Cache-Control", "private, no-store");
    res.headers.set("Cloudflare-CDN-Cache-Control", "no-store");
    return res;
  }

  const headers = new Headers(req.headers);
  if (req.cookies.has(SESSION_COOKIE)) {
    req.cookies.delete(SESSION_COOKIE);
    headers.set("cookie", req.cookies.toString());
  }
  const res = NextResponse.next({ request: { headers } });
  // Next.js 会把动态页面的 Cache-Control 设为 no-store，所以边缘缓存用 Cloudflare 专用头单独声明
  res.headers.set("Cloudflare-CDN-Cache-Control", GUEST_EDGE_CACHE);
  return res;
}

export const config = {
  // 静态资源、图片优化与 API（由后端自己鉴权）不经过这里
  matcher: ["/((?!api/|_next/static|_next/image|favicon.ico|robots.txt|sitemap.xml).*)"],
};
