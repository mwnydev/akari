import "server-only";
import createDOMPurify from "dompurify";
import { JSDOM } from "jsdom";
import { PURIFY_CONFIG } from "./purify-config";

// 服务端渲染的楼层也先过 DOMPurify 再写进页面（DOMPurify 需要 DOM，这里用 jsdom 提供）。
const purify = createDOMPurify(new JSDOM("").window);

export function sanitizeHtml(html: string): string {
  return purify.sanitize(html, PURIFY_CONFIG);
}
