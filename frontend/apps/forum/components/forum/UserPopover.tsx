import { Badge, Button, UserCard, type TradeReputation } from "@akari/ui";
import { LazyPopover } from "@akari/ui/lazy";
import { UserRound } from "lucide-react";
import type { ReactNode } from "react";

/** 头像、名字点开的用户卡片里显示的内容。 */
export interface PersonCard {
  id: number;
  name: string;
  href: string;
  level: number;
  levelName?: string | undefined;
  avatarUrl?: string | null | undefined;
  /** 只有设计稿传；正式页面的在线点在浏览器里取（`avatarBadge`） */
  online?: boolean | undefined;
  lastSeen?: string | undefined;
  reputation?: TradeReputation | undefined;
  /** 公开的拍卖违约记录（「拍卖违约 N 次（近 12 个月）」），没有时不显示 */
  auctionRecord?: string | undefined;
  role?: string | undefined;
  /** 卡片下方的数据格（加入、帖子、获赞……），有才显示 */
  stats?: [label: string, value: string][] | undefined;
}

/** 点头像或名字弹出用户卡片：名片 + 论坛数据 + 去主页。触发元素本身就是按钮，键盘可达。 */
export function UserPopover({
  user,
  label,
  className,
  avatarBadge,
  actions,
  children,
}: {
  user: PersonCard;
  /** 触发按钮的可读名称，默认「{用户名} 的名片」 */
  label?: string;
  className?: string;
  avatarBadge?: ReactNode;
  /** 卡片底部的操作；默认是「个人主页」 */
  actions?: ReactNode;
  children: ReactNode;
}) {
  return (
    <LazyPopover
      trigger={
        <button
          type="button"
          aria-label={label ?? `${user.name} 的名片`}
          className={
            className ??
            "shrink-0 rounded-full focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
          }
        >
          {children}
        </button>
      }
    >
      <UserCard
        name={user.name}
        avatar={user.avatarUrl ?? undefined}
        level={user.level}
        levelName={user.levelName}
        online={user.online}
        avatarBadge={avatarBadge}
        lastSeen={user.lastSeen}
        reputation={user.reputation}
        href={user.href}
      />
      {user.auctionRecord ? (
        <p
          className="mt-2 rounded-md bg-warning-soft px-2 py-1 text-xs"
          data-testid="auction-record"
        >
          {user.auctionRecord}
        </p>
      ) : null}
      {user.role ? (
        <Badge tone="accent" className="mt-2">
          {user.role}
        </Badge>
      ) : null}
      {user.stats?.length ? (
        <dl className="mt-3 grid grid-cols-3 gap-2 text-center">
          {user.stats.map(([k, v]) => (
            <div key={k} className="rounded-md bg-surface-2 py-1.5">
              <dt className="text-xs text-fg-muted">{k}</dt>
              <dd className="amount text-sm font-medium">{v}</dd>
            </div>
          ))}
        </dl>
      ) : null}
      <div className="mt-3 flex gap-2">
        {actions ?? (
          <Button asChild size="sm" variant="secondary" className="flex-1">
            <a href={user.href}>
              <UserRound size={16} aria-hidden />
              个人主页
            </a>
          </Button>
        )}
      </div>
    </LazyPopover>
  );
}
