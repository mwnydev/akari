import { cn } from "@akari/ui";
import type { Board, BoardColor } from "../_mock/data";

const COLORS: Record<BoardColor, string> = {
  red: "bg-board-red",
  blue: "bg-board-blue",
  green: "bg-board-green",
  orange: "bg-board-orange",
  purple: "bg-board-purple",
  teal: "bg-board-teal",
};

/** 板块色块：每个板块一种颜色，只作装饰，旁边总有板块名。 */
export function BoardSquare({ color, className }: { color: BoardColor; className?: string }) {
  return (
    <span aria-hidden className={cn("inline-block size-2.5 shrink-0", COLORS[color], className)} />
  );
}

/** 色块 + 灰色板块名（列表行、主题标题下）。 */
export function BoardMark({ board, href }: { board: Board; href?: string }) {
  const body = (
    <>
      <BoardSquare color={board.color} />
      {board.name}
    </>
  );
  const cls = "inline-flex items-center gap-1.5 text-fg-muted";
  return href ? (
    <a href={href} className={cn(cls, "hover:text-fg")}>
      {body}
    </a>
  ) : (
    <span className={cls}>{body}</span>
  );
}
