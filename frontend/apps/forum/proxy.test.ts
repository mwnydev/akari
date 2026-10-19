import { NextRequest } from "next/server";
import { beforeAll, describe, expect, it } from "vitest";
import { GUEST_EDGE_CACHE, proxy } from "./proxy";

// 会话 Cookie 只由 forum-api 签发（host-only，ADR 0012）；Next.js 这一层只读不写，任何响应都不能带 Set-Cookie。
// 向量同 lib/signed-cookie.test.ts（后端 COOKIE_SECRET = "ab" × 64，签名密钥是前 32 字节）。
const SID = "GEi62EMUlu28DVX+jSiSHeI4AZRsNAZ94oeYYxk/KWc=3oMsdeagrgd5OQIw1233bA";

function request(cookie?: string): NextRequest {
  return new NextRequest("https://akari.cc/t/1", cookie ? { headers: { cookie } } : {});
}

describe("proxy", () => {
  beforeAll(() => {
    process.env.COOKIE_SIGNING_KEY = "ab".repeat(32);
  });

  it("never sets a cookie, for guests, forged and valid sessions", async () => {
    for (const cookie of [undefined, "sid=forged", `sid=${SID}; did=x`]) {
      const res = await proxy(request(cookie));
      expect(res.headers.get("set-cookie"), cookie).toBeNull();
      expect(res.cookies.getAll(), cookie).toEqual([]);
    }
  });

  it("renders a valid session privately and a forged one as a cacheable guest", async () => {
    const valid = await proxy(request(`sid=${SID}`));
    expect(valid.headers.get("cache-control")).toBe("private, no-store");
    expect(valid.headers.get("cloudflare-cdn-cache-control")).toBe("no-store");
    const forged = await proxy(request("sid=forged"));
    expect(forged.headers.get("cloudflare-cdn-cache-control")).toBe(GUEST_EDGE_CACHE);
    expect(GUEST_EDGE_CACHE).toBe(
      "public, max-age=60, stale-while-revalidate=30, stale-if-error=300",
    );
  });
});
