// 在线状态（plan 第 7 节）：页面上所有在线点共用一个仓库。用到的用户攒一批（≤ 100 个）一次查询，
// 页面在前台时每 60 秒刷新；私信 / 订单里的实时推送（subscribed、presence 事件）直接写进来。
import type { Schemas } from "@akari/api-client";
import { api, unwrap } from "./browser-api";

export type Presence = Schemas["Presence"];

const BATCH_MS = 50;
const REFRESH_MS = 60_000;
/** 后端一次最多查 100 人 */
const MAX_IDS = 100;

const known = new Map<number, Presence>();
const watchers = new Map<number, Set<(p: Presence) => void>>();
let batchTimer: ReturnType<typeof setTimeout> | undefined;
let refreshTimer: ReturnType<typeof setInterval> | undefined;
const pending = new Set<number>();

/** 写入一条在线状态（推送或查询结果），通知关注这个人的组件。 */
export function setPresence(p: Presence) {
  known.set(p.user_id, p);
  for (const fn of watchers.get(p.user_id) ?? []) fn(p);
}

async function fetchBatch(ids: number[]) {
  for (let i = 0; i < ids.length; i += MAX_IDS) {
    const chunk = ids.slice(i, i + MAX_IDS);
    try {
      const rows = await unwrap(
        api.GET("/api/v1/users/presence", { params: { query: { ids: chunk.join(",") } } }),
      );
      rows.forEach(setPresence);
    } catch {
      // 在线点只是锦上添花：查询失败就不显示，下一轮再试
    }
  }
}

function flush() {
  batchTimer = undefined;
  const ids = [...pending];
  pending.clear();
  if (ids.length) void fetchBatch(ids);
}

function refreshAll() {
  if (document.visibilityState !== "visible") return;
  void fetchBatch([...watchers.keys()]);
}

/** 关注一个人的在线状态：已知的立即回调一次，并在下一批里查询。返回取消关注。 */
export function watchPresence(userId: number, fn: (p: Presence) => void): () => void {
  let set = watchers.get(userId);
  if (!set) watchers.set(userId, (set = new Set()));
  set.add(fn);
  const now = known.get(userId);
  if (now) fn(now);
  pending.add(userId);
  batchTimer ??= setTimeout(flush, BATCH_MS);
  refreshTimer ??= setInterval(refreshAll, REFRESH_MS);
  return () => {
    set.delete(fn);
    if (set.size === 0) watchers.delete(userId);
    if (watchers.size === 0) {
      clearInterval(refreshTimer);
      refreshTimer = undefined;
    }
  };
}

/** 不在线时的「最近活跃」文案；隐身用户只有日期。 */
export function lastSeenText(p: Presence, now = Date.now()): string | null {
  if (p.online) return null;
  if (p.last_seen_at) {
    const minutes = Math.floor((now - Date.parse(p.last_seen_at)) / 60_000);
    if (minutes < 1) return "刚刚";
    if (minutes < 60) return `${minutes} 分钟前`;
    if (minutes < 24 * 60) return `${Math.floor(minutes / 60)} 小时前`;
    return `${Math.floor(minutes / (24 * 60))} 天前`;
  }
  return p.last_seen_date ?? null;
}
