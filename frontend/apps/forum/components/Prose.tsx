import { cn } from "@akari/ui";
import { PROSE_CLASS } from "../lib/prose";
import { sanitizeHtml } from "../lib/sanitize.server";

/** 帖子正文：后端已用 ammonia 过滤，写进页面前再过一次 DOMPurify（plan 第 11 节）。 */
export function Prose({ html, className }: { html: string; className?: string }) {
  return (
    <div
      className={cn(PROSE_CLASS, className)}
      dangerouslySetInnerHTML={{ __html: sanitizeHtml(html) }}
    />
  );
}
