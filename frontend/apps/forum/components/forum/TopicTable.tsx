import { Avatar, AvatarStack, Badge, cn } from "@akari/ui";
import { EyeOff, Lock, Pin } from "lucide-react";
import Link from "next/link";
import type { ReactNode } from "react";
import { formatCount } from "../../lib/display";
import { BoardMark, type BoardRef } from "./BoardMark";
import { KindBadge } from "./KindBadge";
import { UserPopover, type PersonCard } from "./UserPopover";

export type TopicKind = "normal" | "trade" | "auction";

/** 主题列表一行要显示的内容（正式页面由接口数据换算，设计稿由模拟数据换算）。 */
export interface TopicRowView {
  id: number;
  title: string;
  href: string;
  kind: TopicKind;
  /** 跨板块列表才显示板块 */
  board?: BoardRef | undefined;
  author: PersonCard;
  /** 最近发言的人，第一位是作者；接口没给时不显示参与者列 */
  participants: { name: string; avatarUrl?: string | null | undefined }[];
  replies: number;
  views: number;
  /** 最后活动：机读时间与短写法（「3 分钟」） */
  activityIso: string;
  activity: string;
  pinned: boolean;
  featured: boolean;
  locked: boolean;
  /** 隐藏帖只有管理人员看得到 */
  hidden: boolean;
  /** 未读回复数（登录用户） */
  unread?: number | undefined;
}

/** 回复数达到这些档位时换成「热度」色（从高到低匹配）。 */
const HEAT: [number, string][] = [
  [150, "text-heat-3"],
  [100, "text-heat-2"],
  [50, "text-heat-1"],
];

function heatClass(replies: number): string {
  const hit = HEAT.find(([min]) => replies >= min);
  return hit ? `font-medium ${hit[1]}` : "text-fg-muted";
}

const th = "px-2 py-2 font-normal";
const numCell = "amount hidden px-2 text-right md:table-cell";

/** 主题表格：没有外框，表头是灰色小字，行之间 1px 分隔线。子元素是若干 TopicRow。 */
export function TopicListFrame({ label, children }: { label: string; children: ReactNode }) {
  return (
    <table className="w-full border-collapse">
      <caption className="sr-only">{label}</caption>
      <thead className="hidden md:table-header-group">
        <tr className="border-b border-border text-left text-sm text-fg-muted">
          <th scope="col" className={th}>
            主题
          </th>
          <th scope="col" className={`${th} w-40`}>
            参与者
          </th>
          <th scope="col" className={`${th} w-16 text-right`}>
            回复
          </th>
          <th scope="col" className={`${th} w-16 text-right`}>
            浏览
          </th>
          <th scope="col" className={`${th} w-20 text-right`}>
            活动
          </th>
        </tr>
      </thead>
      <tbody>{children}</tbody>
    </table>
  );
}

/**
 * 主题列表一行（Discourse 式表格行，自有实现）。
 * 桌面端：主题、参与者、回复、浏览、活动五列；移动端：左侧作者头像，右侧回复数与最后活动。
 * `info` 接在板块与标签后面（交易帖的价格、拍卖的当前价等）。
 */
export function TopicRow({ topic: t, info }: { topic: TopicRowView; info?: ReactNode }) {
  return (
    <tr className="border-b border-border" data-testid="topic-row">
      <td className="py-3 pr-2 md:pl-2">
        <div className="flex gap-3">
          <span className="md:hidden">
            <UserPopover user={t.author}>
              <Avatar name={t.author.name} src={t.author.avatarUrl ?? undefined} size={36} />
            </UserPopover>
          </span>
          <div className="min-w-0">
            <Link
              href={t.href}
              className="rounded-sm text-[17px] leading-snug break-words text-fg visited:text-fg-muted focus-visible:outline-2 focus-visible:outline-accent"
            >
              {t.pinned ? (
                <>
                  <Pin size={15} aria-hidden className="mr-1 inline align-[-2px] text-fg-muted" />
                  <span className="sr-only">置顶：</span>
                </>
              ) : null}
              {t.locked ? (
                <>
                  <Lock size={15} aria-hidden className="mr-1 inline align-[-2px] text-fg-muted" />
                  <span className="sr-only">已锁定：</span>
                </>
              ) : null}
              {t.hidden ? (
                <>
                  <EyeOff size={15} aria-hidden className="mr-1 inline align-[-2px] text-danger" />
                  <span className="sr-only">已隐藏：</span>
                </>
              ) : null}
              {t.title}
            </Link>
            {t.unread ? (
              <span className="amount ml-1.5 inline-flex h-5 min-w-5 items-center justify-center rounded-full bg-accent-fill px-1.5 align-[2px] text-xs font-medium text-accent-fg">
                {t.unread}
                <span className="sr-only"> 条新回复</span>
              </span>
            ) : null}
            <p className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-sm text-fg-muted">
              {t.board ? <BoardMark board={t.board} /> : null}
              {t.kind !== "normal" ? <KindBadge kind={t.kind} /> : null}
              {t.featured ? <Badge tone="success">精华</Badge> : null}
              {info}
            </p>
          </div>
        </div>
      </td>
      <td className="hidden px-2 md:table-cell">
        {t.participants.length ? (
          <AvatarStack
            people={t.participants.map((p) => ({
              name: p.name,
              src: p.avatarUrl ?? undefined,
            }))}
          />
        ) : null}
      </td>
      <td className={cn(numCell, heatClass(t.replies))}>{formatCount(t.replies)}</td>
      <td className={cn(numCell, "text-fg-muted")}>{formatCount(t.views)}</td>
      <td className="hidden pr-2 pl-2 text-right text-sm whitespace-nowrap text-fg-muted md:table-cell">
        <time dateTime={t.activityIso}>{t.activity}</time>
      </td>
      <td className="py-3 text-right align-top text-sm md:hidden">
        <span className={cn("amount block", heatClass(t.replies))}>
          {formatCount(t.replies)}
          <span className="sr-only"> 条回复</span>
        </span>
        <time dateTime={t.activityIso} className="block text-xs whitespace-nowrap text-fg-muted">
          {t.activity}
        </time>
      </td>
    </tr>
  );
}
