/** 板块色块，对应 globals.css 里的 --color-board-* 令牌（只作装饰，旁边总有板块名）。 */
export type BoardColor = "red" | "blue" | "green" | "orange" | "purple" | "teal";

const ORDER: BoardColor[] = ["red", "blue", "purple", "green", "teal", "orange"];

/** 接口的固定色板（迁移 20261202000001_forum_color 的 CHECK）→ 令牌名。 */
const PALETTE: Record<string, BoardColor> = {
  "#d1453b": "red",
  "#2f7fd8": "blue",
  "#2e9d5c": "green",
  "#e08e1b": "orange",
  "#8059c9": "purple",
  "#1d9bb0": "teal",
};

/** 板块设置了颜色就用它；没设置（或不在色板里）时按排序位置轮换，同一个板块在各页颜色一致。 */
export function boardColor(forum: {
  position: number;
  id: number;
  color?: string | null | undefined;
}): BoardColor {
  const set = forum.color ? PALETTE[forum.color] : undefined;
  if (set) return set;
  const i = Number.isSafeInteger(forum.position) && forum.position >= 0 ? forum.position : forum.id;
  return ORDER[i % ORDER.length] ?? "blue";
}
