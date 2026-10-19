import { FORUM_ORIGIN } from "./domains";

/**
 * 签完回论坛的地址：只接受论坛站内路径（以单个 / 开头），拼上论坛域名；其他一律回论坛首页，
 * 防止链接里的 return 把用户带到钓鱼站。
 */
export function forumReturnUrl(path: string | null): string {
  if (!path || !/^\/(?![/\\])[^\s]*$/.test(path)) return FORUM_ORIGIN;
  const url = new URL(path, FORUM_ORIGIN);
  return url.origin === new URL(FORUM_ORIGIN).origin ? url.href : FORUM_ORIGIN;
}
