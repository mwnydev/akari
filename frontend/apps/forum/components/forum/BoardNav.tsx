import { cn } from "@akari/ui";
import { Bookmark, Clock, Flame, type LucideIcon } from "lucide-react";
import type { BoardColor } from "../../lib/board-color";
import { BoardSquare } from "./BoardMark";

const item =
  "flex h-8 items-center gap-2.5 rounded-sm px-2.5 text-[15px] text-fg hover:bg-surface-2 " +
  "focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent";

const heading = "px-2.5 pb-1 text-xs font-medium text-fg-muted";

const ICONS = { latest: Clock, hot: Flame, bookmarks: Bookmark } satisfies Record<
  string,
  LucideIcon
>;

export interface NavShortcut {
  key: string;
  href: string;
  label: string;
  icon: keyof typeof ICONS;
}

export interface NavBoard {
  slug: string;
  name: string;
  color: BoardColor;
  href: string;
  /** 今日新帖（接口有这项数据时才传） */
  today?: number;
  /** 子板块，缩进显示 */
  children?: NavBoard[];
}

function BoardItem({ b, active, nested }: { b: NavBoard; active: string; nested?: boolean }) {
  return (
    <li>
      <a
        href={b.href}
        aria-current={active === b.slug ? "page" : undefined}
        className={cn(item, active === b.slug && "bg-surface-2", nested && "pl-7 text-sm")}
      >
        <BoardSquare color={b.color} className={nested ? "size-2" : "mx-[3px]"} />
        <span className="min-w-0 flex-1 truncate">{b.name}</span>
        {b.today ? (
          <span className="amount text-xs text-fg-muted" aria-label={`今日 ${b.today} 帖`}>
            {b.today}
          </span>
        ) : null}
      </a>
      {b.children?.length ? (
        <ul>
          {b.children.map((c) => (
            <BoardItem key={c.slug} b={c} active={active} nested />
          ))}
        </ul>
      ) : null}
    </li>
  );
}

/**
 * 侧栏（桌面常驻、移动端抽屉共用）：直接放在页面底色上，灰色小标题分组，当前项浅灰底。
 * `active` 是快捷入口的 key 或板块 slug。用普通链接：抽屉里点了就整页跳转，抽屉随之关闭。
 */
export function BoardNav({
  shortcuts,
  boards,
  active,
}: {
  shortcuts: NavShortcut[];
  boards: NavBoard[];
  active: string;
}) {
  return (
    <nav aria-label="板块" className="space-y-5">
      <div>
        <h2 className={heading}>浏览</h2>
        <ul>
          {shortcuts.map(({ key, href, label, icon }) => {
            const Icon = ICONS[icon];
            return (
              <li key={key}>
                <a
                  href={href}
                  aria-current={active === key ? "page" : undefined}
                  className={cn(item, active === key && "bg-surface-2")}
                >
                  <Icon size={16} aria-hidden className="shrink-0 text-fg-muted" />
                  {label}
                </a>
              </li>
            );
          })}
        </ul>
      </div>
      {boards.length ? (
        <div>
          <h2 className={heading}>板块</h2>
          <ul>
            {boards.map((b) => (
              <BoardItem key={b.slug} b={b} active={active} />
            ))}
          </ul>
        </div>
      ) : null}
    </nav>
  );
}
