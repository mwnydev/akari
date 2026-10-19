import { cn } from "@akari/ui";
import type { ReactNode } from "react";

/**
 * 论坛页面骨架（Discourse 式布局，自有实现）：顶栏 + 左侧栏 + 正文。桌面端侧栏常驻，窄屏收进顶栏汉堡按钮打开的抽屉。
 * `banner` 放在顶栏之上（设计稿提示条）；`className` 是正文区域的额外样式（主题页的分栏网格等）。
 */
export function ForumShell({
  header,
  nav,
  banner,
  className,
  children,
}: {
  header: ReactNode;
  nav: ReactNode;
  banner?: ReactNode;
  className?: string | undefined;
  children: ReactNode;
}) {
  return (
    <>
      {banner}
      {header}
      <div className="mx-auto max-w-[90rem] px-3 sm:px-4 lg:grid lg:grid-cols-[14rem_minmax(0,1fr)] lg:gap-8">
        <aside className="hidden lg:block">
          <div className="sticky top-14 max-h-[calc(100dvh-3.5rem)] overflow-y-auto py-5">
            {nav}
          </div>
        </aside>
        <main className={cn("min-w-0 pt-4 pb-10 lg:pt-5", className)}>{children}</main>
      </div>
    </>
  );
}
