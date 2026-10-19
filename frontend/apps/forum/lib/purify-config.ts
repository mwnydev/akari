import type { Config, DOMPurify } from "dompurify";
import { linkMentions } from "./mentions";

/**
 * 前端 DOMPurify 二次过滤（plan 第 11 节）：与后端 ammonia 的白名单一致（backend/crates/content），
 * 后端漏放的任何标签或属性在这里还会再被拦一次。`rel` 是 ammonia 自己给链接加的。
 */
export const PURIFY_CONFIG = {
  ALLOWED_TAGS: [
    "p",
    "br",
    "strong",
    "b",
    "em",
    "i",
    "s",
    "u",
    "code",
    "pre",
    "blockquote",
    "ul",
    "ol",
    "li",
    "h2",
    "h3",
    "h4",
    "hr",
    "a",
    "img",
    "span",
  ],
  ALLOWED_ATTR: [
    "href",
    "rel",
    "src",
    "alt",
    "width",
    "height",
    "start",
    "class",
    "data-type",
    "data-id",
    "data-label",
  ],
  ALLOW_DATA_ATTR: false,
} satisfies Config;

/** 过滤帖子 HTML，再把 @提及换成个人主页链接（lib/mentions）。服务端渲染与浏览器共用。 */
export function sanitizePost(purify: DOMPurify, html: string): string {
  const fragment = purify.sanitize(html, { ...PURIFY_CONFIG, RETURN_DOM_FRAGMENT: true });
  linkMentions(fragment);
  const box = fragment.ownerDocument.createElement("div");
  box.append(fragment);
  return box.innerHTML;
}
