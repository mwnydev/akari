// 列表与楼层共用的显示规则。
import type { Schemas } from "@akari/api-client";

export type UserCardView = Schemas["UserCardView"];

/** 还没设置用户名时显示「用户 {id}」（后端 UserCardView 的约定）。 */
export function userName(u: { id: number; username?: string | null | undefined }): string {
  return u.username || `用户 ${u.id}`;
}

/** 个人主页地址：有用户名用用户名，否则用数字 ID。 */
export function profileHref(u: { id: number; username?: string | null | undefined }): string {
  return `/u/${encodeURIComponent(u.username || String(u.id))}`;
}

/** 接口里的用户名片列表 → 按 ID 查找；缺失时给出占位名片，页面不因个别用户缺失而报错。 */
export function cardLookup(users: UserCardView[]): (id: number) => UserCardView {
  const byId = new Map(users.map((u) => [u.id, u]));
  return (id) => byId.get(id) ?? { id, username: null, level: 1 };
}

const MAX_MENTION_SUGGESTIONS = 8;

/** @提及候选：按名字包含输入过滤。后端还没有用户搜索接口，候选是本页出现过的用户。 */
export function filterMentionable(users: UserCardView[], query: string): UserCardView[] {
  const q = query.trim().toLowerCase();
  return users
    .filter((u) => userName(u).toLowerCase().includes(q))
    .slice(0, MAX_MENTION_SUGGESTIONS);
}

// 服务端渲染、游客页整页缓存，所以时间按站点时区格式化，而不是访问者的时区。
const SITE_TIME_ZONE = "Asia/Shanghai";
const dateTime = new Intl.DateTimeFormat("zh-CN", {
  timeZone: SITE_TIME_ZONE,
  year: "numeric",
  month: "2-digit",
  day: "2-digit",
  hour: "2-digit",
  minute: "2-digit",
  hourCycle: "h23",
});

/** RFC 3339 → 「2026-10-19 09:04」。 */
export function formatTime(iso: string): string {
  const parts = Object.fromEntries(
    dateTime.formatToParts(new Date(iso)).map((p) => [p.type, p.value]),
  );
  return `${parts.year}-${parts.month}-${parts.day} ${parts.hour}:${parts.minute}`;
}

/** 秒数 → 整天数文案（交易期限都是整天）。 */
export function formatDays(seconds: number): string {
  const days = seconds / 86_400;
  return Number.isInteger(days) ? `${days} 天` : `${Math.round(seconds / 3600)} 小时`;
}
