// 导航栏的通知未读数：启动时查一次；收到推送先加一，再稍后向后端核对（推送与查询可能重叠，以后端为准）；
// 重连 / 补拉时、别的标签页标为已读时更新；推送不可用时每 30 秒轮询。由 NavLive 按需加载，不进首屏 JS。
import { api, unwrap } from "../browser-api";
import { UNREAD_CAP } from "../badge";
import { acquireHub, releaseHub } from "./react";

const RECHECK_MS = 1_500;
const POLL_MS = 30_000;

export function watchUnread(onCount: (count: number) => void): () => void {
  const hub = acquireHub();
  let count = 0;
  let lastReadId = 0;
  let stopped = false;
  let recheck: ReturnType<typeof setTimeout> | undefined;
  let poll: ReturnType<typeof setInterval> | undefined;

  const set = (n: number) => {
    count = Math.min(UNREAD_CAP, n);
    onCount(count);
  };
  const refresh = async () => {
    try {
      const u = await unwrap(api.GET("/api/v1/notifications/unread"));
      if (stopped) return;
      lastReadId = u.last_read_id;
      set(u.count);
    } catch {
      // 未登录或网络问题：保持现有数字，下次再查
    }
  };
  const schedule = () => {
    clearTimeout(recheck);
    recheck = setTimeout(() => void refresh(), RECHECK_MS);
  };
  const setPolling = (on: boolean) => {
    clearInterval(poll);
    poll = on ? setInterval(() => void refresh(), POLL_MS) : undefined;
  };

  const offEvents = hub.on((e) => {
    switch (e.type) {
      case "notification":
        if (e.notification.id > lastReadId && !e.notification.read) {
          set(count + 1);
          schedule();
        }
        return;
      case "notifications_read":
        lastReadId = Math.max(lastReadId, e.last_read_id);
        set(e.count);
        return;
      case "resync":
        void refresh();
        return;
      default:
        return;
    }
  });
  const offStatus = hub.onStatus((s) => setPolling(s === "polling"));
  setPolling(hub.status === "polling");
  void refresh();

  return () => {
    stopped = true;
    clearTimeout(recheck);
    setPolling(false);
    offEvents();
    offStatus();
    releaseHub();
  };
}
