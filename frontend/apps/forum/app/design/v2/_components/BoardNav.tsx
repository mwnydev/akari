import { cn } from "@akari/ui";
import { Bookmark, Clock, Flame, type LucideIcon } from "lucide-react";
import { boards } from "../_mock/data";
import { BoardSquare } from "./BoardMark";

const item =
  "flex h-8 items-center gap-2.5 rounded-sm px-2.5 text-[15px] text-fg hover:bg-surface-2 " +
  "focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent";

const heading = "px-2.5 pb-1 text-xs font-medium text-fg-muted";

const SHORTCUTS: { key: string; href: string; label: string; Icon: LucideIcon }[] = [
  { key: "home", href: "/design/v2", label: "最新", Icon: Clock },
  { key: "hot", href: "/design/v2", label: "热门", Icon: Flame },
  { key: "bookmarks", href: "#", label: "我的收藏", Icon: Bookmark },
];

/** 侧栏（桌面常驻、移动端抽屉共用）：直接放在页面底色上，灰色小标题分组，当前项浅灰底。 */
export function BoardNav({ active }: { active: string }) {
  const current = (key: string) => (active === key ? "page" : undefined);
  return (
    <nav aria-label="板块" className="space-y-5">
      <div>
        <h2 className={heading}>浏览</h2>
        <ul>
          {SHORTCUTS.map(({ key, href, label, Icon }) => (
            <li key={key}>
              <a
                href={href}
                aria-current={current(key)}
                className={cn(item, active === key && "bg-surface-2")}
              >
                <Icon size={16} aria-hidden className="shrink-0 text-fg-muted" />
                {label}
              </a>
            </li>
          ))}
        </ul>
      </div>
      <div>
        <h2 className={heading}>板块</h2>
        <ul>
          {boards.map((b) => (
            <li key={b.slug}>
              <a
                href={b.slug === "trade" ? "/design/v2/board" : "#"}
                aria-current={current(b.slug)}
                className={cn(item, active === b.slug && "bg-surface-2")}
              >
                <BoardSquare color={b.color} className="mx-[3px]" />
                <span className="min-w-0 flex-1 truncate">{b.name}</span>
                {b.today > 0 ? (
                  <span className="amount text-xs text-fg-muted" aria-label={`今日 ${b.today} 帖`}>
                    {b.today}
                  </span>
                ) : null}
              </a>
            </li>
          ))}
        </ul>
      </div>
    </nav>
  );
}
