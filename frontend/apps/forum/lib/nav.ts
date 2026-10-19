import type { Me } from "@akari/api-client";
import type { NavBoard, NavShortcut } from "../components/forum/BoardNav";
import { boardColor } from "./board-color";
import { profileHref } from "./display";
import type { ForumNode, ForumView } from "./forums";
import { boardHref } from "./links";

/** 侧栏「浏览」分组。 */
export const SHORTCUTS: NavShortcut[] = [
  { key: "home", href: "/", label: "最新", icon: "latest" },
  { key: "hot", href: "/?tab=hot", label: "热门", icon: "hot" },
];

export function boardRef(f: ForumView) {
  return { name: f.name, color: boardColor(f), href: boardHref(f.slug) };
}

/** 板块树 → 侧栏条目。 */
export function navBoards(tree: ForumNode[]): NavBoard[] {
  const one = (f: ForumView): NavBoard => ({ slug: f.slug, ...boardRef(f) });
  return tree.map((f) => ({ ...one(f), children: f.children.map(one) }));
}

/** 未设置用户名时显示邮箱 @ 前的部分（只有本人看得到自己的邮箱）。 */
export function viewerName(me: Me): string {
  return me.username || me.email.split("@")[0] || me.email;
}

export function headerViewer(me: Me) {
  return { name: viewerName(me), profileHref: profileHref(me) };
}
