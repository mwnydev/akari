import type { Config } from "dompurify";

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
