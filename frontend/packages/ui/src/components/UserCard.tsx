import { cn } from "../cn";
import { Avatar } from "./Avatar";
import { Badge } from "./Badge";

export interface TradeReputation {
  /** 成功率，千分比整数（986 = 98.6%），避免浮点 */
  successPermille: number;
  deals: number;
  disputesLost: number;
  /** 卖家平均回复用时（分钟）；未知时不显示 */
  replyMinutes?: number | undefined;
}

export interface UserCardProps {
  name: string;
  avatar?: string | undefined;
  level: number;
  levelName?: string | undefined;
  online?: boolean | undefined;
  /** 最近活跃文案（不在线时显示），如「5 分钟前」 */
  lastSeen?: string | undefined;
  reputation?: TradeReputation | undefined;
  size?: "sm" | "md";
  /** 个人主页链接；钱包、后台等不在论坛域名下的页面不传 */
  href?: string | undefined;
  className?: string;
}

/** 信誉一行：不满 10 笔显示「新卖家」（plan 第 7 节）。 */
export function reputationLine(r: TradeReputation): string {
  if (r.deals < 10) return `新卖家 · 已成交 ${r.deals} 笔`;
  const pct = `${Math.floor(r.successPermille / 10)}.${r.successPermille % 10}%`;
  const parts = [`成功率 ${pct}`, `成交 ${r.deals} 笔`, `争议败诉 ${r.disputesLost} 次`];
  if (r.replyMinutes !== undefined) parts.push(`通常 ${r.replyMinutes} 分钟内回复`);
  return parts.join(" · ");
}

/** 用户名片：头像 + 在线点 + 等级 + 信誉。全站只此一份实现。 */
export function UserCard({
  name,
  avatar,
  level,
  levelName,
  online,
  lastSeen,
  reputation,
  size = "md",
  href,
  className,
}: UserCardProps) {
  return (
    <div className={cn("flex min-w-0 items-center gap-3", className)}>
      <Avatar name={name} src={avatar} online={online} size={size === "sm" ? 32 : 44} />
      <div className="min-w-0">
        <div className="flex items-center gap-1.5">
          {href ? (
            <a href={href} className="truncate font-medium hover:underline">
              {name}
            </a>
          ) : (
            <span className="truncate font-medium">{name}</span>
          )}
          <Badge tone="accent">
            Lv{level}
            {levelName ? ` ${levelName}` : ""}
          </Badge>
          {!online && lastSeen ? (
            <span className="shrink-0 text-xs text-fg-muted">最近活跃：{lastSeen}</span>
          ) : null}
        </div>
        {reputation ? (
          <p className="amount truncate text-sm text-fg-muted">{reputationLine(reputation)}</p>
        ) : null}
      </div>
    </div>
  );
}
