import { Badge } from "@akari/ui";
import { EyeOff, Lock, Pin } from "lucide-react";
import type { ReactNode } from "react";
import { formatCount } from "../../lib/display";
import { BoardMark, type BoardRef } from "./BoardMark";
import { KindBadge } from "./KindBadge";
import type { TopicKind } from "./TopicTable";

export interface TopicHeaderView {
  title: string;
  kind: TopicKind;
  board: BoardRef;
  pinned: boolean;
  locked: boolean;
  featured: boolean;
  hidden: boolean;
  replies: number;
  views: number;
  /** 参与人数（设计稿）；接口只给列表行头像用的前几位，不是总人数，正式页面不传 */
  participants?: number | undefined;
}

/**
 * 主题页标题区：大标题，下面一行板块与标签，再下面是统计。`actions` 在标题右侧（管理人员的「管理」菜单）；
 * `extra` 接在统计后面（页码等）。
 */
export function TopicHeader({
  topic: t,
  actions,
  extra,
}: {
  topic: TopicHeaderView;
  actions?: ReactNode;
  extra?: ReactNode;
}) {
  return (
    <header className="mb-3">
      <div className="flex items-start gap-3">
        <h1 className="min-w-0 flex-1 text-[1.625rem] leading-snug font-medium break-words sm:text-[1.75rem]">
          {t.pinned ? (
            <>
              <Pin size={20} aria-hidden className="mr-1.5 inline align-[-1px] text-fg-muted" />
              <span className="sr-only">置顶：</span>
            </>
          ) : null}
          {t.locked ? (
            <>
              <Lock size={20} aria-hidden className="mr-1.5 inline align-[-1px] text-fg-muted" />
              <span className="sr-only">已锁定：</span>
            </>
          ) : null}
          {t.title}
        </h1>
        {actions}
      </div>
      <div className="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-sm">
        <BoardMark board={t.board} link />
        {t.kind !== "normal" ? <KindBadge kind={t.kind} /> : null}
        {t.featured ? <Badge tone="success">精华</Badge> : null}
        {t.hidden ? (
          <Badge tone="danger">
            <EyeOff size={12} aria-hidden />
            已隐藏
          </Badge>
        ) : null}
      </div>
      <p className="amount mt-1.5 text-sm text-fg-muted">
        {t.replies} 回复 · {formatCount(t.views)} 浏览
        {t.participants !== undefined ? ` · ${t.participants} 人参与` : null}
        {extra}
      </p>
    </header>
  );
}
