import "server-only";
import type { Schemas } from "@akari/api-client";
import { cache } from "react";
import { load, serverApi } from "./api";

export type ForumView = Schemas["ForumView"];

/** 访问者能看到的全部板块（同一次渲染内只请求一次）。 */
export const listForums = cache(async () => load((await serverApi()).GET("/api/v1/forums")));

export interface ForumNode extends ForumView {
  children: ForumView[];
}

/** 平铺的板块按 `parent_id` 组成两级树（plan 第 4 节「多级板块」；更深的层级挂到最近的可见祖先下）。 */
export function forumTree(forums: ForumView[]): ForumNode[] {
  const byId = new Map(forums.map((f) => [f.id, f]));
  const roots = new Map<number, ForumNode>();
  const rootOf = (f: ForumView): ForumView => {
    let cur = f;
    const seen = new Set<number>();
    while (cur.parent_id != null && byId.has(cur.parent_id) && !seen.has(cur.id)) {
      seen.add(cur.id);
      cur = byId.get(cur.parent_id) as ForumView;
    }
    return cur;
  };
  const sorted = [...forums].sort((a, b) => a.position - b.position || a.id - b.id);
  for (const f of sorted) {
    if (rootOf(f).id === f.id) roots.set(f.id, { ...f, children: [] });
  }
  for (const f of sorted) {
    const root = rootOf(f);
    if (root.id !== f.id) roots.get(root.id)?.children.push(f);
  }
  return [...roots.values()];
}
