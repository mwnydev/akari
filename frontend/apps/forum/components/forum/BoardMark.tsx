import { cn } from "@akari/ui";
import Link from "next/link";
import type { BoardColor } from "../../lib/board-color";

const COLORS: Record<BoardColor, string> = {
  red: "bg-board-red",
  blue: "bg-board-blue",
  green: "bg-board-green",
  orange: "bg-board-orange",
  purple: "bg-board-purple",
  teal: "bg-board-teal",
};

/** 列表、标题下、侧栏里显示的板块。 */
export interface BoardRef {
  name: string;
  color: BoardColor;
  href: string;
}

/** 板块色块：每个板块一种颜色，只作装饰，旁边总有板块名。 */
export function BoardSquare({ color, className }: { color: BoardColor; className?: string }) {
  return (
    <span aria-hidden className={cn("inline-block size-2.5 shrink-0", COLORS[color], className)} />
  );
}

/** 色块 + 灰色板块名（列表行、主题标题下）；`link` 为假时只显示不跳转。 */
export function BoardMark({ board, link = false }: { board: BoardRef; link?: boolean }) {
  const body = (
    <>
      <BoardSquare color={board.color} />
      {board.name}
    </>
  );
  const cls = "inline-flex items-center gap-1.5 text-fg-muted";
  return link ? (
    <Link href={board.href} className={cn(cls, "rounded-sm hover:text-fg")}>
      {body}
    </Link>
  ) : (
    <span className={cls}>{body}</span>
  );
}
