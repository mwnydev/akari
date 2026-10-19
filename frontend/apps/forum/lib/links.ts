/** 主题某页（第 1 页不带 ?page，便于判断是否同一页、也是游客缓存的同一个地址）。 */
export function topicHref(id: number, page = 1, floor?: number): string {
  const base = page > 1 ? `/t/${id}?page=${page}` : `/t/${id}`;
  return floor ? `${base}#floor-${floor}` : base;
}

/** 板块某页；`cursor` 用于第 200 页之后。 */
export function boardHref(slug: string, page = 1, cursor?: string): string {
  const base = `/f/${encodeURIComponent(slug)}`;
  if (cursor) return `${base}?cursor=${encodeURIComponent(cursor)}`;
  return page > 1 ? `${base}?page=${page}` : base;
}

/** 登录页，登录后回到 `next`（只接受站内路径，见 app/login/safe-next.ts）。 */
export function loginHref(next: string): string {
  return `/login?next=${encodeURIComponent(next)}`;
}
