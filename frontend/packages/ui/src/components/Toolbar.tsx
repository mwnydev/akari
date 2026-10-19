import type { HTMLAttributes, ReactNode } from "react";
import { cn } from "../cn";

/** 浮动玻璃工具栏（胶囊形）。玻璃只用在这类固定层。 */
export function Toolbar({ className, ...props }: HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      role="toolbar"
      className={cn("glass flex items-center gap-1 rounded-pill p-1 shadow-float", className)}
      {...props}
    />
  );
}

export interface NavLink {
  href: string;
  label: string;
  active?: boolean;
}

/** 顶部玻璃导航：品牌 + 链接 + 右侧插槽。链接用普通 <a>，各应用可用 Link 包装。 */
export function GlassNav({
  brand,
  links,
  right,
}: {
  brand: ReactNode;
  links: NavLink[];
  right?: ReactNode;
}) {
  return (
    <div className="sticky top-0 z-30 px-3 pt-3 sm:px-4">
      <nav
        aria-label="主导航"
        className="glass mx-auto flex h-12 max-w-5xl items-center gap-2 rounded-pill pr-1.5 pl-4 shadow-float"
      >
        <span className="mr-1 shrink-0 font-semibold">{brand}</span>
        <div className="flex min-w-0 flex-1 items-center gap-0.5 overflow-x-auto [scrollbar-width:none]">
          {links.map((l) => (
            <a
              key={l.href}
              href={l.href}
              aria-current={l.active ? "page" : undefined}
              className={cn(
                "shrink-0 rounded-pill px-3 py-1 text-sm",
                l.active ? "bg-surface-2 font-medium" : "text-fg-muted hover:text-fg",
              )}
            >
              {l.label}
            </a>
          ))}
        </div>
        {right ? <div className="flex shrink-0 items-center gap-1">{right}</div> : null}
      </nav>
    </div>
  );
}
