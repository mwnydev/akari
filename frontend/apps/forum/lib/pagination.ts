import { POSTS_PER_PAGE } from "./public-config";

/** 楼层所在页码。 */
export function floorPage(floor: number): number {
  return Math.max(1, Math.ceil(floor / POSTS_PER_PAGE));
}

export type PageItem = number | "gap";

/**
 * 分页条上要显示的页码：首页、末页、当前页前后各 `radius` 页，其余折叠为 "gap"。
 * 只差一页的折叠直接显示那一页（「1 … 3」没有意义）。
 */
export function pageWindow(current: number, total: number, radius = 2): PageItem[] {
  if (total <= 1) return [1];
  const keep = new Set([1, total]);
  for (let p = current - radius; p <= current + radius; p++) {
    if (p >= 1 && p <= total) keep.add(p);
  }
  const sorted = [...keep].sort((a, b) => a - b);
  const out: PageItem[] = [];
  let prev = 0;
  for (const p of sorted) {
    if (p - prev === 2) out.push(prev + 1);
    else if (p - prev > 2) out.push("gap");
    out.push(p);
    prev = p;
  }
  return out;
}

/** 解析 `?page=`、`?floor=` 这类查询参数：缺省、非正整数都当 1。 */
export function parsePositiveInt(raw: string | string[] | undefined): number {
  const v = Array.isArray(raw) ? raw[0] : raw;
  if (!v || !/^\d{1,9}$/.test(v)) return 1;
  return Math.max(1, Number(v));
}

/**
 * 首页与板块的「热门」：`?hot=` 是热门下一页的游标，`?tab=hot` 直接打开热门标签页。只有这两种情况才取热门
 * （排名单独一次查询，打开「最新」时不必多请求）；`active` 为假时页面只放一个切过去再加载的标签页。
 */
export function hotRequest(sp: Record<string, string | string[] | undefined>): {
  active: boolean;
  cursor: string | undefined;
} {
  const cursor = typeof sp.hot === "string" && sp.hot ? sp.hot : undefined;
  return { active: cursor !== undefined || sp.tab === "hot", cursor };
}
