import { Avatar, Badge } from "@akari/ui";
import type { ReactNode } from "react";
import { UserPopover, type PersonCard } from "./UserPopover";

/**
 * 一层楼（Discourse 式：左侧头像、右侧正文，楼层之间 1px 分隔线，没有卡片；窄屏头像收进标题行）。
 * 头像和名字都能打开用户卡片；`children` 是正文与操作（FloorContent）。
 * `badges` 叠在大 / 小两个头像上（正式页面在浏览器里取的在线点）。
 */
export function Floor({
  floor,
  author,
  badges,
  isOp,
  timeIso,
  time,
  timeFull,
  edited,
  children,
}: {
  floor: number;
  author: PersonCard;
  badges?: { lg: ReactNode; sm: ReactNode };
  /** 楼主，名字旁标「楼主」 */
  isOp: boolean;
  timeIso: string;
  /** 短写法（「3 小时」），悬停显示 `timeFull` */
  time: string;
  timeFull?: string | undefined;
  /** 「已编辑」的悬停说明（编辑时间）；没编辑过不传 */
  edited?: string | undefined;
  children: ReactNode;
}) {
  const avatar = (size: number, badge: ReactNode) => (
    <Avatar
      name={author.name}
      src={author.avatarUrl ?? undefined}
      size={size}
      online={author.online}
      badge={badge}
    />
  );
  return (
    <article
      id={`floor-${floor}`}
      data-floor={floor}
      data-testid="floor"
      aria-label={`#${floor} ${author.name}`}
      className="flex scroll-mt-20 gap-3 border-t border-border py-4 sm:gap-4"
    >
      <div className="hidden sm:block">
        <UserPopover user={author} avatarBadge={badges?.lg}>
          {avatar(45, badges?.lg)}
        </UserPopover>
      </div>
      <div className="min-w-0 flex-1">
        <header className="flex flex-wrap items-center gap-x-1.5 gap-y-0.5">
          <span className="mr-0.5 sm:hidden">
            <UserPopover user={author} avatarBadge={badges?.sm}>
              {avatar(32, badges?.sm)}
            </UserPopover>
          </span>
          <UserPopover
            user={author}
            label={`${author.name}，查看名片`}
            avatarBadge={badges?.lg}
            className="rounded-sm font-semibold hover:underline focus-visible:outline-2 focus-visible:outline-accent"
          >
            {author.name}
          </UserPopover>
          <Badge tone="accent">Lv{author.level}</Badge>
          {author.role ? <Badge>{author.role}</Badge> : null}
          {isOp ? <Badge tone="outline">楼主</Badge> : null}
          <span className="ml-auto flex items-center gap-2 text-sm text-fg-muted">
            {edited ? <span title={edited}>已编辑</span> : null}
            <time dateTime={timeIso} title={timeFull}>
              {time}
            </time>
            <a href={`#floor-${floor}`} className="amount rounded-sm hover:text-fg">
              #{floor}
            </a>
          </span>
        </header>
        {children}
      </div>
    </article>
  );
}

/** 楼层流：没有外框，楼层之间是分隔线。 */
export function FloorStream({ children, label }: { children: ReactNode; label: string }) {
  return (
    <section aria-label={label} className="border-b border-border">
      {children}
    </section>
  );
}
