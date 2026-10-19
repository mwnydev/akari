// 搜索页与顶栏搜索框共用的规则（后端 GET /api/v1/search）。

export type SearchType = "post" | "topic" | "user";

export const SEARCH_TYPES: { value: SearchType; label: string }[] = [
  { value: "post", label: "帖子" },
  { value: "topic", label: "主题" },
  { value: "user", label: "用户" },
];

/** 与后端 `forum_db::search::MAX_QUERY_CHARS` 一致：更长的部分后端也会截掉。 */
export const MAX_QUERY_CHARS = 64;
/** 顶栏输入停顿多久才查联想结果，以及至少几个字才查（游客每分钟 20 次搜索的限额经不起逐字请求）。 */
export const SUGGEST_DEBOUNCE_MS = 300;
export const SUGGEST_MIN_CHARS = 2;
export const SUGGEST_LIMIT = 6;

export interface SearchParams {
  q: string;
  type: SearchType;
  board: string | undefined;
  cursor: string | undefined;
}

function first(v: string | string[] | undefined): string | undefined {
  return Array.isArray(v) ? v[0] : v;
}

/** 去首尾空白、按字符（不是 UTF-16 码元）截到上限。 */
export function normalizeQuery(raw: string): string {
  return Array.from(raw.trim()).slice(0, MAX_QUERY_CHARS).join("").trim();
}

/** 地址栏参数 → 搜索条件；未知的类型当作帖子。 */
export function parseSearchParams(sp: Record<string, string | string[] | undefined>): SearchParams {
  const type = first(sp.type);
  const board = first(sp.board);
  const cursor = first(sp.cursor);
  return {
    q: normalizeQuery(first(sp.q) ?? ""),
    type: SEARCH_TYPES.some((t) => t.value === type) ? (type as SearchType) : "post",
    board: board || undefined,
    cursor: cursor || undefined,
  };
}

/** 搜索页地址；默认值（帖子、全部板块、第一页）不写进地址。 */
export function searchHref(p: Partial<SearchParams> & { q: string }): string {
  const qs = new URLSearchParams({ q: p.q });
  if (p.type && p.type !== "post") qs.set("type", p.type);
  if (p.board) qs.set("board", p.board);
  if (p.cursor) qs.set("cursor", p.cursor);
  return `/search?${qs.toString()}`;
}

/** 顶栏联想要不要发请求：太短的不查。 */
export function suggestQuery(raw: string): string | null {
  const q = normalizeQuery(raw);
  return Array.from(q).length >= SUGGEST_MIN_CHARS ? q : null;
}
