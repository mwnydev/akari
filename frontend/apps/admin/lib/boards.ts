import type { Schemas } from "@akari/api-client";

export type Board = Schemas["ForumView"];
export type BoardUpdate = Schemas["ForumUpdateRequest"];

/**
 * 板块色板：与后端迁移 20261202000001 的 CHECK、`forum_db::forums::PALETTE` 和论坛 globals.css 的
 * `--color-board-*` 一致（boards.test.ts 核对论坛那份）。`null` = 不指定，论坛按排序位置轮换。
 */
export const PALETTE: { hex: string; label: string }[] = [
  { hex: "#d1453b", label: "红" },
  { hex: "#2f7fd8", label: "蓝" },
  { hex: "#2e9d5c", label: "绿" },
  { hex: "#e08e1b", label: "橙" },
  { hex: "#8059c9", label: "紫" },
  { hex: "#1d9bb0", label: "青" },
];

export interface BoardValues {
  name: string;
  description: string;
  /** 色板里的 hex；"" = 按位置轮换 */
  color: string;
  position: number;
  allowTrade: boolean;
}

export function boardValues(b: Board): BoardValues {
  return {
    name: b.name,
    description: b.description,
    color: b.color ?? "",
    position: b.position,
    allowTrade: b.allow_trade,
  };
}

/**
 * 只提交改了的字段（后端不改没给的字段）；没有改动返回 null。`tradeChanged`：改了「能否发交易帖」，
 * 后端要求 5 分钟内再验证过。
 */
export function boardPatch(
  b: Board,
  v: BoardValues,
): { body: BoardUpdate; tradeChanged: boolean } | null {
  const body: BoardUpdate = {};
  const name = v.name.trim();
  const description = v.description.trim();
  if (name !== b.name) body.name = name;
  if (description !== b.description) body.description = description;
  if (v.color !== (b.color ?? "")) body.color = v.color === "" ? null : v.color;
  if (v.position !== b.position) body.position = v.position;
  const tradeChanged = v.allowTrade !== b.allow_trade;
  if (tradeChanged) body.allow_trade = v.allowTrade;
  return Object.keys(body).length ? { body, tradeChanged } : null;
}
