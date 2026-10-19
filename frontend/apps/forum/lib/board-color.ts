/** 板块色块，对应 globals.css 里的 --color-board-* 令牌（只作装饰，旁边总有板块名）。 */
export type BoardColor = "red" | "blue" | "green" | "orange" | "purple" | "teal";

const ORDER: BoardColor[] = ["red", "blue", "purple", "green", "teal", "orange"];

/** 接口里的板块还没有颜色字段：按排序位置轮换，同一个板块在各页颜色一致。 */
export function boardColor(forum: { position: number; id: number }): BoardColor {
  const i = Number.isSafeInteger(forum.position) && forum.position >= 0 ? forum.position : forum.id;
  return ORDER[i % ORDER.length] ?? "blue";
}
