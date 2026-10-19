/** 「#123」或纯数字按用户 ID；否则按用户名前缀搜索。 */
export function parseUserQuery(raw: string): { id: number } | { prefix: string } | null {
  const t = raw.trim();
  if (!t) return null;
  const m = /^#?(\d{1,15})$/.exec(t);
  return m ? { id: Number(m[1]) } : { prefix: t };
}
