import { cn } from "@akari/ui";
import type { ReactNode } from "react";
import { BoardNav } from "./BoardNav";
import { SiteHeader } from "./SiteHeader";

const PAGES = [
  { href: "/design/v2", label: "首页" },
  { href: "/design/v2/board", label: "板块页" },
  { href: "/design/v2/trade", label: "交易帖" },
  { href: "/design/v2/auction", label: "拍卖帖" },
  { href: "/design/v2/topic", label: "普通帖" },
  { href: "/design", label: "设计系统" },
];

/** 设计稿提示条，兼做各页之间的切换。 */
function MockupBar({ page }: { page: string }) {
  return (
    <div className="border-b border-warning/30 bg-warning-soft px-4 py-1 text-xs text-fg">
      <nav
        aria-label="设计稿页面"
        className="mx-auto flex max-w-[90rem] flex-wrap items-center gap-x-3 gap-y-0.5"
      >
        <span>设计稿 · 模拟数据，按钮不会产生真实操作</span>
        {PAGES.map((p) => (
          <a
            key={p.href}
            href={p.href}
            aria-current={p.href === page ? "page" : undefined}
            className="underline-offset-2 hover:underline aria-[current=page]:font-semibold aria-[current=page]:underline"
          >
            {p.label}
          </a>
        ))}
      </nav>
    </div>
  );
}

/**
 * 论坛页面骨架（Discourse 式布局，自有实现）：顶栏 + 左侧栏 + 正文。桌面端侧栏常驻，窄屏收进顶栏汉堡按钮打开的抽屉。
 * `page` 是当前设计稿路径，`active` 是侧栏高亮项（home 或板块 slug）。
 */
export function ForumShell({
  page,
  active,
  className,
  children,
}: {
  page: string;
  active: string;
  /** 正文区域的额外样式（主题页的分栏网格等） */
  className?: string;
  children: ReactNode;
}) {
  return (
    <>
      <MockupBar page={page} />
      <SiteHeader active={active} />
      <div className="mx-auto max-w-[90rem] px-3 sm:px-4 lg:grid lg:grid-cols-[14rem_minmax(0,1fr)] lg:gap-8">
        <aside className="hidden lg:block">
          <div className="sticky top-14 max-h-[calc(100dvh-3.5rem)] overflow-y-auto py-5">
            <BoardNav active={active} />
          </div>
        </aside>
        <main className={cn("min-w-0 pt-4 pb-10 lg:pt-5", className)}>{children}</main>
      </div>
    </>
  );
}
