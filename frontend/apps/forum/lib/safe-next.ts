import { WALLET_ORIGIN } from "./public-config";

/**
 * 登录后跳回的地址，防开放重定向：站内相对路径（单个 / 开头），或者钱包中心（配置的 WALLET_ORIGIN，
 * 源必须完全相同，不带用户名密码）里的页面——钱包的「去登录」登录完回到钱包。其他一律回首页。
 */
export function safeNext(raw: string | string[] | undefined, walletOrigin = WALLET_ORIGIN): string {
  const v = Array.isArray(raw) ? raw[0] : raw;
  if (!v) return "/";
  if (v.startsWith("/")) {
    return v.startsWith("//") || v.startsWith("/\\") ? "/" : v;
  }
  let url: URL;
  let wallet: URL;
  try {
    url = new URL(v);
    wallet = new URL(walletOrigin);
  } catch {
    return "/";
  }
  // 字符串本身也要以钱包源 + "/" 开头：不让解析器的宽容写法（反斜杠、空白、大小写变体）混过去
  if (
    url.origin !== wallet.origin ||
    url.username !== "" ||
    url.password !== "" ||
    !v.startsWith(`${wallet.origin}/`) ||
    /[\s\\]/.test(v)
  ) {
    return "/";
  }
  return url.href;
}

/** `safeNext` 的结果是不是钱包地址（要整页跳转，不能交给 Next 路由）。 */
export function isExternalNext(next: string): boolean {
  return !next.startsWith("/");
}
