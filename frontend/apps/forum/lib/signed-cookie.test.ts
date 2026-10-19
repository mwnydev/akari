import { describe, expect, it } from "vitest";
import { importSigningKey, verifySignedValue } from "./signed-cookie";

// 向量取自本地运行的 forum-api（COOKIE_SECRET = "ab" × 64）真实登录返回的 Set-Cookie。
const SECRET = "ab".repeat(64);
const SID = "GEi62EMUlu28DVX+jSiSHeI4AZRsNAZ94oeYYxk/KWc=3oMsdeagrgd5OQIw1233bA";
// did 由 axum-extra SignedCookieJar 写出，Set-Cookie 里是 URL 编码形式
const DID_ENCODED =
  "2jWE%2FAgUHJSXcEJA6JH8fdJyyO0vO56+08PRRDfGZT8%3Dadef781d1c89f523fd78e692fe7c43eb";

describe("verifySignedValue", () => {
  it("accepts the session cookie issued by the backend", async () => {
    const key = await importSigningKey(SECRET);
    expect(await verifySignedValue(key, SID)).toBe("3oMsdeagrgd5OQIw1233bA");
  });

  it("accepts the URL-decoded device cookie", async () => {
    const key = await importSigningKey(SECRET);
    expect(await verifySignedValue(key, decodeURIComponent(DID_ENCODED))).toBe(
      "adef781d1c89f523fd78e692fe7c43eb",
    );
  });

  it("rejects a tampered value, a forged digest and the wrong key", async () => {
    const key = await importSigningKey(SECRET);
    expect(await verifySignedValue(key, SID.slice(0, -1) + "B")).toBeNull();
    expect(
      await verifySignedValue(key, "A".repeat(43) + "=" + "3oMsdeagrgd5OQIw1233bA"),
    ).toBeNull();
    const other = await importSigningKey("cd".repeat(64));
    expect(await verifySignedValue(other, SID)).toBeNull();
  });

  it("rejects malformed values without throwing", async () => {
    const key = await importSigningKey(SECRET);
    for (const v of ["", "short", SID.slice(0, 44), "!".repeat(60), "%%%".repeat(20)]) {
      expect(await verifySignedValue(key, v)).toBeNull();
    }
  });

  it("refuses a short or non-hex secret", async () => {
    await expect(importSigningKey("ab".repeat(63))).rejects.toThrow();
    await expect(importSigningKey("zz".repeat(64))).rejects.toThrow();
  });
});
