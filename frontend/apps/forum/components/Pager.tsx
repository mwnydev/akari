import { Button } from "@akari/ui";
import { ChevronLeft, ChevronRight } from "lucide-react";
import Link from "next/link";
import type { ReactNode } from "react";
import { pageWindow } from "../lib/pagination";

function PageLink({
  href,
  current,
  children,
  label,
}: {
  href: string;
  current?: boolean;
  children: ReactNode;
  label?: string;
}) {
  return (
    <Button asChild size="sm" variant={current ? "primary" : "secondary"} className="min-w-9 px-2">
      <Link href={href} aria-current={current ? "page" : undefined} aria-label={label}>
        {children}
      </Link>
    </Button>
  );
}

/**
 * 页码分页条。`total` 已知时显示首末页与当前页附近（主题楼层）；
 * 未知时（板块列表只知道有没有下一页）显示从第 1 页到当前页附近，再加下一页。
 */
export function Pager({
  current,
  total,
  hasNext,
  href,
  after,
}: {
  current: number;
  /** 总页数；未知时传 undefined 并用 hasNext */
  total?: number | undefined;
  hasNext?: boolean;
  href: (page: number) => string;
  /** 页码之后的额外入口（如游标翻页） */
  after?: ReactNode;
}) {
  const last = total ?? (hasNext ? current + 1 : current);
  if (last <= 1 && !after) return null;
  const items = pageWindow(current, last);
  return (
    <nav aria-label="分页" className="flex flex-wrap items-center justify-center gap-1 py-2">
      {current > 1 ? (
        <PageLink href={href(current - 1)} label="上一页">
          <ChevronLeft size={16} aria-hidden />
        </PageLink>
      ) : null}
      {items.map((p, i) =>
        p === "gap" ? (
          <span key={`gap-${i}`} className="px-1 text-fg-muted" aria-hidden>
            …
          </span>
        ) : (
          <PageLink key={p} href={href(p)} current={p === current}>
            {p}
          </PageLink>
        ),
      )}
      {current < last ? (
        <PageLink href={href(current + 1)} label="下一页">
          <ChevronRight size={16} aria-hidden />
        </PageLink>
      ) : null}
      {after}
    </nav>
  );
}
