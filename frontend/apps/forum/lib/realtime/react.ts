"use client";

// 页面里用实时推送的 hooks。整个标签页共用一个 RealtimeHub：第一个使用者启动它，最后一个离开后稍等再关闭
//（客户端路由切换时组件先卸载再挂载，不必断开重连）。
import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import type { RealtimeEvent, Topic } from "./events";
import { RealtimeHub, socketUrl, type ChannelLike } from "./hub";
import type { RealtimeStatus, SocketLike } from "./socket";

const IDLE_CLOSE_MS = 3_000;

let hub: RealtimeHub | null = null;
let users = 0;
let idleTimer: ReturnType<typeof setTimeout> | undefined;

function getHub(): RealtimeHub {
  if (hub) return hub;
  const h = new RealtimeHub({
    url: socketUrl(location),
    // 浏览器的 WebSocket 回调带 `this` 与具体事件类型，结构上就是 SocketLike
    createSocket:
      typeof WebSocket === "undefined"
        ? null
        : (url) => new WebSocket(url) as unknown as SocketLike,
    openChannel:
      typeof BroadcastChannel === "undefined"
        ? null
        : () => new BroadcastChannel("akari-realtime") as unknown as ChannelLike,
    locks: navigator.locks ?? null,
  });
  const retry = () => {
    if (document.visibilityState === "visible") h.retryNow();
  };
  window.addEventListener("online", retry);
  document.addEventListener("visibilitychange", retry);
  // 页面离开时撤回订阅、交出连接（主页面的锁浏览器也会释放）；从往返缓存恢复时再启动
  window.addEventListener("pagehide", () => h.stop());
  window.addEventListener("pageshow", (e) => {
    if (e.persisted && users > 0) h.start();
  });
  return (hub = h);
}

/** 开始使用实时连接（引用计数）；用完调用 releaseHub。hooks 之外的模块（如导航栏未读数）直接用。 */
export function acquireHub(): RealtimeHub {
  users++;
  clearTimeout(idleTimer);
  const h = getHub();
  h.start();
  return h;
}

export function releaseHub() {
  users--;
  if (users > 0) return;
  idleTimer = setTimeout(() => {
    if (users === 0) hub?.stop();
  }, IDLE_CLOSE_MS);
}

/** 组件挂载期间保持连接，并接收全部推送事件。`onEvent` 可以每次渲染都变。 */
export function useRealtimeEvents(onEvent: (e: RealtimeEvent) => void) {
  const latest = useRef(onEvent);
  useEffect(() => {
    latest.current = onEvent;
  });
  useEffect(() => {
    const h = acquireHub();
    const off = h.on((e) => latest.current(e));
    return () => {
      off();
      releaseHub();
    };
  }, []);
}

function subscribeStatus(notify: () => void): () => void {
  const h = acquireHub();
  const off = h.onStatus(notify);
  return () => {
    off();
    releaseHub();
  };
}

/** 连接状态；`polling` 时页面应定时用 REST 拉取。服务端渲染时按 `connecting`。 */
export function useRealtimeStatus(): RealtimeStatus {
  return useSyncExternalStore(
    subscribeStatus,
    () => hub?.status ?? "connecting",
    () => "connecting",
  );
}

/** 打开会话 / 订单期间订阅它；返回「发送正在输入」。 */
export function useTopic(topic: Topic): () => void {
  const key = JSON.stringify(topic);
  const [current, setCurrent] = useState<RealtimeHub | null>(null);
  useEffect(() => {
    const h = acquireHub();
    setCurrent(h);
    const unsubscribe = h.subscribe(JSON.parse(key) as Topic);
    return () => {
      unsubscribe();
      releaseHub();
      setCurrent(null);
    };
  }, [key]);
  return () => current?.typing(JSON.parse(key) as Topic);
}

/** 本页的本地事件也转给其他标签页（如通知已读）。 */
export function broadcastLocal(e: RealtimeEvent) {
  hub?.broadcast(e);
}
