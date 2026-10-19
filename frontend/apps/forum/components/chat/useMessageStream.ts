"use client";

import { errorMessage } from "@akari/api-client";
import { useCallback, useEffect, useRef, useState } from "react";
import { lastId, mergeById } from "../../lib/chat";
import type { RealtimeEvent } from "../../lib/realtime/events";
import { useRealtimeEvents, useRealtimeStatus } from "../../lib/realtime/react";

export interface StreamPage<T> {
  messages: T[];
  has_more: boolean;
}

/** 翻页参数：都不给取最新一页；`before` 取更早的一页；`after` 取之后的（补拉）。 */
export type StreamQuery = { before?: number; after?: number };

export interface MessageStreamOptions<T extends { id: number }> {
  /** 换会话 / 订单时重新加载 */
  id: number;
  load: (id: number, q: StreamQuery) => Promise<StreamPage<T>>;
  /** 推送事件里属于这个会话 / 订单的新消息 */
  pick: (e: RealtimeEvent, id: number) => T | null;
  /** 推送不可用时的轮询间隔 */
  pollMs: number;
}

/** 补拉时一次最多连拉几页（每页 50 条），防止异常数据导致无限循环。 */
const MAX_CATCH_UP_PAGES = 20;

/**
 * 私信与订单聊天共用的消息流：先取最新一页，向上翻页取更早的；推送来的新消息按 ID 合并；
 * 连上 / 重连（resync）或轮询时用 `after` 补拉到最新。
 */
export function useMessageStream<T extends { id: number }>({
  id,
  load,
  pick,
  pollMs,
}: MessageStreamOptions<T>) {
  const [messages, setMessages] = useState<T[]>([]);
  const [hasOlder, setHasOlder] = useState(false);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const [state, setState] = useState<{ loading: boolean; error: string | null }>({
    loading: true,
    error: null,
  });
  const current = useRef<{ id: number; messages: T[] }>({ id, messages: [] });
  const catching = useRef<Promise<void> | null>(null);
  const opts = useRef({ load, pick });
  useEffect(() => {
    opts.current = { load, pick };
  });

  const merge = useCallback((forId: number, incoming: T[]) => {
    if (current.current.id !== forId) return;
    const next = mergeById(current.current.messages, incoming);
    current.current.messages = next;
    setMessages(next);
  }, []);

  /** 从最后一条之后补拉到最新（同一时间只跑一个）。 */
  const catchUp = useCallback(async () => {
    if (catching.current) return catching.current;
    const forId = current.current.id;
    const run = (async () => {
      try {
        for (let i = 0; i < MAX_CATCH_UP_PAGES; i++) {
          const after = lastId(current.current.messages);
          const page = await opts.current.load(forId, after ? { after } : {});
          merge(forId, page.messages);
          if (!after) setHasOlder(page.has_more);
          if (!after || !page.has_more) break;
        }
      } catch {
        // 补拉失败不打断聊天：下次推送重连或轮询时再试
      }
    })();
    catching.current = run;
    try {
      await run;
    } finally {
      catching.current = null;
    }
  }, [merge]);

  useEffect(() => {
    current.current = { id, messages: [] };
    setMessages([]);
    setHasOlder(false);
    setState({ loading: true, error: null });
    let cancelled = false;
    opts.current
      .load(id, {})
      .then((page) => {
        if (cancelled) return;
        merge(id, page.messages);
        setHasOlder(page.has_more);
        setState({ loading: false, error: null });
      })
      .catch((e: unknown) => {
        if (!cancelled) setState({ loading: false, error: errorMessage(e) });
      });
    return () => {
      cancelled = true;
    };
  }, [id, merge]);

  useRealtimeEvents((e) => {
    if (e.type === "resync") {
      if (!state.loading) void catchUp();
      return;
    }
    const m = opts.current.pick(e, id);
    if (m) merge(id, [m]);
  });

  const status = useRealtimeStatus();
  useEffect(() => {
    if (status !== "polling" || state.loading || state.error) return;
    const t = setInterval(() => void catchUp(), pollMs);
    return () => clearInterval(t);
  }, [status, state.loading, state.error, pollMs, catchUp]);

  const loadOlder = useCallback(() => {
    const first = current.current.messages[0];
    if (!first) return;
    const forId = current.current.id;
    setLoadingOlder(true);
    opts.current
      .load(forId, { before: first.id })
      .then((page) => {
        merge(forId, page.messages);
        if (current.current.id === forId) setHasOlder(page.has_more);
      })
      .catch(() => {
        // 保留「加载更早的消息」按钮，用户可以再点
      })
      .finally(() => setLoadingOlder(false));
  }, [merge]);

  /** 自己刚发出的消息（接口返回值），不必等推送。 */
  const add = useCallback((m: T) => merge(current.current.id, [m]), [merge]);

  return {
    messages,
    loading: state.loading,
    error: state.error,
    older: { hasMore: hasOlder, loading: loadingOlder, load: loadOlder },
    add,
  };
}
