import { Badge, Button, UserCard } from "@akari/ui";
import { LazyPopover } from "@akari/ui/lazy";
import { MessageCircle, UserPlus } from "lucide-react";
import type { ReactNode } from "react";
import type { MockUser } from "../_mock/data";

/** 点头像或名字弹出用户卡片：名片 + 论坛数据 + 私信 / 关注。触发元素本身就是按钮，键盘可达。 */
export function UserPopover({
  user,
  label,
  className,
  children,
}: {
  user: MockUser;
  /** 触发按钮的可读名称，默认「{用户名} 的名片」 */
  label?: string;
  className?: string;
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
        level={user.level}
        levelName={user.levelName}
        online={user.online}
        lastSeen={user.lastSeen}
        reputation={user.reputation}
        href="#"
      />
      {user.role ? (
        <Badge tone="accent" className="mt-2">
          {user.role}
        </Badge>
      ) : null}
      <dl className="mt-3 grid grid-cols-3 gap-2 text-center">
        {[
          ["加入", user.joined],
          ["帖子", String(user.posts)],
          ["获赞", String(user.likes)],
        ].map(([k, v]) => (
          <div key={k} className="rounded-md bg-surface-2 py-1.5">
            <dt className="text-xs text-fg-muted">{k}</dt>
            <dd className="amount text-sm font-medium">{v}</dd>
          </div>
        ))}
      </dl>
      <div className="mt-3 flex gap-2">
        <Button size="sm" className="flex-1">
          <MessageCircle size={16} aria-hidden />
          私信
        </Button>
        <Button size="sm" variant="secondary" className="flex-1">
          <UserPlus size={16} aria-hidden />
          关注
        </Button>
      </div>
    </LazyPopover>
  );
}
