/** 与后端 `REVOKE_MAX_SPAN` 一致：一次撤销的时间段最长 31 天。 */
export const MAX_REVOKE_SPAN_MS = 31 * 86_400_000;

/**
 * `<input type="datetime-local">` 的两个值（本地时间）→ 接口要的 RFC 3339（UTC）。
 * 不合法、结束不晚于开始或超过 31 天返回 null。
 */
export function revokeWindow(from: string, to: string): { from: string; to: string } | null {
  const a = new Date(from);
  const b = new Date(to);
  if (!from || !to || Number.isNaN(a.getTime()) || Number.isNaN(b.getTime())) return null;
  const span = b.getTime() - a.getTime();
  if (span <= 0 || span > MAX_REVOKE_SPAN_MS) return null;
  return { from: a.toISOString(), to: b.toISOString() };
}
