// 列表与楼层共用的显示规则。
import type { Schemas } from "@akari/api-client";
import type { TradeReputation } from "@akari/ui";

export type UserCardView = Schemas["UserCardView"];

/** 还没设置用户名时显示「用户 {id}」（后端 UserCardView 的约定）。 */
export function userName(u: { id: number; username?: string | null | undefined }): string {
  return u.username || `用户 ${u.id}`;
}

/** 个人主页地址：有用户名用用户名，否则用数字 ID。 */
export function profileHref(u: { id: number; username?: string | null | undefined }): string {
  return `/u/${encodeURIComponent(u.username || String(u.id))}`;
}

/** 还没有任何交易记录的信誉。 */
export const NO_REPUTATION: Schemas["ReputationView"] = {
  success: 0,
  failed: 0,
  settled: 0,
  disputes_lost: 0,
  success_rate_bps: null,
};

/** 接口里的用户名片列表 → 按 ID 查找；缺失时给出占位名片，页面不因个别用户缺失而报错。 */
export function cardLookup(users: UserCardView[]): (id: number) => UserCardView {
  const byId = new Map(users.map((u) => [u.id, u]));
  return (id) => byId.get(id) ?? { id, username: null, level: 1, reputation: NO_REPUTATION };
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

function timeParts(iso: string): Record<string, string> {
  return Object.fromEntries(dateTime.formatToParts(new Date(iso)).map((p) => [p.type, p.value]));
}

/** RFC 3339 → 「2026-10-19 09:04」。 */
export function formatTime(iso: string): string {
  const p = timeParts(iso);
  return `${p.year}-${p.month}-${p.day} ${p.hour}:${p.minute}`;
}

/** 聊天里的时间：今年的省掉年份 →「10-19 09:04」。 */
export function formatShortTime(iso: string, now: Date = new Date()): string {
  const p = timeParts(iso);
  const thisYear = timeParts(now.toISOString()).year;
  const day = p.year === thisYear ? `${p.month}-${p.day}` : `${p.year}-${p.month}-${p.day}`;
  return `${day} ${p.hour}:${p.minute}`;
}

/** 名片上的信誉 → 用户名片组件的格式（成功率万分比 → 千分比；不满 10 笔后端不给成功率）。 */
export function tradeReputation(r: Schemas["ReputationView"]): TradeReputation {
  return {
    successPermille: Math.floor((r.success_rate_bps ?? 0) / 10),
    deals: r.success,
    disputesLost: r.disputes_lost,
  };
}

/** 秒数 → 整天数文案（交易期限都是整天）。 */
export function formatDays(seconds: number): string {
  const days = seconds / 86_400;
  return Number.isInteger(days) ? `${days} 天` : `${Math.round(seconds / 3600)} 小时`;
}
