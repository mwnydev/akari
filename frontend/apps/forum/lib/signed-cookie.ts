// 校验后端签发的签名 Cookie（Rust `cookie` 0.18 crate 的 SignedJar 格式），只用 Web Crypto。
//
// 格式：value = base64(HMAC-SHA256(key, 原值)) + 原值，base64 为标准字母表带填充，固定 44 个字符。
// key = COOKIE_SIGNING_KEY = 后端 COOKIE_SECRET 的前 32 字节（`cookie::Key::from(master)` 直接切分，不做派生）。
// Next.js 只拿签名这一半：后一半是加密密钥，前端用不到，也就不该能拿到（最小权限）。
// 这里只判断「是否为我们签发的 Cookie」；会话是否仍然有效由后端 /me 决定。

const DIGEST_B64_LEN = 44;
const DIGEST_LEN = 32;
const SIGNING_KEY_LEN = 32;

/** 从 COOKIE_SIGNING_KEY（正好 32 字节十六进制）导入用于校验的 HMAC 密钥；其他长度一律拒绝。 */
export async function importSigningKey(keyHex: string): Promise<CryptoKey> {
  if (!/^[0-9a-fA-F]{64}$/.test(keyHex)) {
    throw new Error(
      "COOKIE_SIGNING_KEY must be exactly 32 bytes of hex (first half of COOKIE_SECRET)",
    );
  }
  const raw = new Uint8Array(SIGNING_KEY_LEN);
  for (let i = 0; i < SIGNING_KEY_LEN; i++) {
    raw[i] = Number.parseInt(keyHex.slice(i * 2, i * 2 + 2), 16);
  }
  return crypto.subtle.importKey("raw", raw, { name: "HMAC", hash: "SHA-256" }, false, ["verify"]);
}

function decodeBase64(s: string): Uint8Array<ArrayBuffer> | null {
  if (!/^[A-Za-z0-9+/]{43}=$/.test(s)) return null;
  const bin = atob(s);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out.length === DIGEST_LEN ? out : null;
}

/**
 * 校验签名 Cookie 的值（已做 URL 解码的形式）。通过返回原值，否则返回 null。
 * 比较由 `crypto.subtle.verify` 完成，与时间无关。
 */
export async function verifySignedValue(
  key: CryptoKey,
  cookieValue: string,
): Promise<string | null> {
  if (cookieValue.length <= DIGEST_B64_LEN) return null;
  const digest = decodeBase64(cookieValue.slice(0, DIGEST_B64_LEN));
  if (!digest) return null;
  const value = cookieValue.slice(DIGEST_B64_LEN);
  const ok = await crypto.subtle.verify("HMAC", key, digest, new TextEncoder().encode(value));
  return ok ? value : null;
}
