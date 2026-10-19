import { Badge } from "@akari/ui";
import { Lock, Pin } from "lucide-react";
import { board, user, VIEWER_ID, type TopicSummary } from "../_mock/data";
import { BoardMark } from "./BoardMark";
import { formatCount } from "./format";
import { KindBadge } from "./KindBadge";
import { ModMenu } from "./ModMenu";

/** 主题页标题区：大标题，下面一行板块与标签，再下面是统计；管理人员右侧多一个「管理」菜单。 */
export function TopicHeader({ topic: t }: { topic: TopicSummary }) {
  const canModerate = user(VIEWER_ID).role !== undefined;
  return (
    <header className="mb-3">
      <div className="flex items-start gap-3">
        <h1 className="min-w-0 flex-1 text-[1.625rem] leading-snug font-medium sm:text-[1.75rem]">
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
        {canModerate ? <ModMenu board={t.board} author={user(t.authorId).name} /> : null}
      </div>
      <div className="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-sm">
        <BoardMark board={board(t.board)} href="/design/v2/board" />
        {t.trade ? <KindBadge kind="trade" /> : null}
        {t.auction ? <KindBadge kind="auction" /> : null}
        {t.featured ? <Badge tone="success">精华</Badge> : null}
      </div>
      <p className="amount mt-1.5 text-sm text-fg-muted">
        {t.replies} 回复 · {formatCount(t.views)} 浏览 · {t.participants.length} 人参与
      </p>
    </header>
  );
}
