import type { ReactNode } from "react";
import { BoardNav } from "../../../../components/forum/BoardNav";
import { ForumShell } from "../../../../components/forum/ForumShell";
import { SiteHeader } from "../../../../components/forum/SiteHeader";
import { countDot } from "../../../../components/forum/styles";
import { user, VIEWER_ID } from "../_mock/data";
import { mockNavBoards, mockShortcuts } from "../_mock/views";

const PAGES = [
  { href: "/design/v2", label: "首页" },
  { href: "/design/v2/board", label: "板块页" },
  { href: "/design/v2/trade", label: "交易帖" },
  { href: "/design/v2/auction", label: "拍卖帖" },
  { href: "/design/v2/topic", label: "普通帖" },
  { href: "/design", label: "设计系统" },
];

/** 设计稿里的未读通知数。 */
const UNREAD_NOTIFICATIONS = 3;

/** 设计稿提示条，兼做各页之间的切换。 */
function MockupBar({ page }: { page: string }) {
  return (
    <div className="border-b border-warning/30 bg-warning-soft px-4 py-1 text-xs text-fg">
      <nav
        aria-label="设计稿页面"
        className="mx-auto flex max-w-[90rem] flex-wrap items-center gap-x-3 gap-y-0.5"
      >
        <span>设计稿 · 正式组件 + 模拟数据，按钮不会产生真实操作</span>
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
 * 设计稿页面骨架：正式的 ForumShell / SiteHeader / BoardNav，填模拟数据，固定显示「已登录」状态。
 * `page` 是当前设计稿路径，`active` 是侧栏高亮项（home 或板块 slug）。
 */
export function MockShell({
  page,
  active,
  className,
  children,
}: {
  page: string;
  active: string;
  className?: string | undefined;
  children: ReactNode;
}) {
  const viewer = user(VIEWER_ID);
  const nav = <BoardNav shortcuts={mockShortcuts} boards={mockNavBoards} active={active} />;
  return (
    <ForumShell
      banner={<MockupBar page={page} />}
      header={
        <SiteHeader
          homeHref="/design/v2"
          nav={nav}
          viewer={{ name: viewer.name, profileHref: "#" }}
          unread={
            <>
              <span aria-hidden className={countDot}>
                {UNREAD_NOTIFICATIONS}
              </span>
              <span className="sr-only">，{UNREAD_NOTIFICATIONS} 条未读</span>
            </>
          }
        />
      }
      nav={nav}
      className={className}
    >
      {children}
    </ForumShell>
  );
}
