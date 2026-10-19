import DOMPurify from "dompurify";
import { PURIFY_CONFIG } from "./purify-config";

/** 浏览器里插入接口返回的 HTML（编辑历史等）之前再过滤一次。 */
export function sanitizeHtml(html: string): string {
  return DOMPurify.sanitize(html, PURIFY_CONFIG);
}
