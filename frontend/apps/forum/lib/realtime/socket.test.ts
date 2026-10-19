import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backoffDelay, RECONNECT_BACKOFF } from "./backoff";
import type { RealtimeEvent } from "./events";
import {
  PING_INTERVAL_MS,
  PONG_TIMEOUT_MS,
  POLL_AFTER_FAILURES,
  RealtimeSocket,
  type RealtimeStatus,
  type SocketLike,
} from "./socket";

function must<T>(v: T | null | undefined): T {
  if (v == null) throw new Error("missing");
  return v;
}

class FakeSocket implements SocketLike {
  readyState = 0;
  sent: unknown[] = [];
  closed = false;
  onopen: SocketLike["onopen"] = null;
  onmessage: SocketLike["onmessage"] = null;
  onclose: SocketLike["onclose"] = null;
  onerror: SocketLike["onerror"] = null;

  send(data: string) {
    this.sent.push(JSON.parse(data));
  }
  close() {
    this.closed = true;
  }
  /** 服务端接受连接并发出 ready */
  accept() {
    this.readyState = 1;
    this.onopen?.({});
    this.server({ type: "ready", user_id: 1, heartbeat_secs: 30 });
  }
  server(msg: object) {
    this.onmessage?.({ data: JSON.stringify(msg) });
  }
  drop() {
    this.readyState = 3;
    this.onclose?.({});
  }
}

function setup() {
  const sockets: FakeSocket[] = [];
  const events: RealtimeEvent[] = [];
  const statuses: RealtimeStatus[] = [];
  const s = new RealtimeSocket({
    url: "ws://t/api/v1/ws",
    create: () => {
      const f = new FakeSocket();
      sockets.push(f);
      return f;
    },
    onEvent: (e) => events.push(e),
    onStatus: (st) => statuses.push(st),
    random: () => 0.5,
  });
  return { s, sockets, events, statuses, last: () => must(sockets.at(-1)) };
}

describe("backoffDelay", () => {
  it("grows exponentially with jitter inside [d/2, d) and is capped", () => {
    const b = { baseMs: 1000, maxMs: 8000 };
    expect(backoffDelay(0, b, () => 0)).toBe(500);
    expect(backoffDelay(0, b, () => 0.999)).toBe(999);
    expect(backoffDelay(2, b, () => 0)).toBe(2000);
    expect(backoffDelay(10, b, () => 0)).toBe(4000);
    expect(backoffDelay(10, b, () => 0.999)).toBeLessThan(8000);
  });

  it("never overflows for huge attempt counts", () => {
    expect(backoffDelay(10_000, RECONNECT_BACKOFF, () => 1)).toBe(RECONNECT_BACKOFF.maxMs);
  });
});

describe("RealtimeSocket", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("subscribes after ready, forwards events and asks pages to resync", () => {
    const { s, last, events, statuses } = setup();
    s.subscribe("c:7");
    s.start();
    expect(statuses).toEqual(["connecting"]);
    last().accept();
    expect(statuses).toEqual(["connecting", "open"]);
    expect(last().sent).toEqual([{ type: "subscribe", conversation_id: 7 }]);
    last().server({ type: "pong" });
    last().server({ type: "notification", notification: { id: 3 } });
    last().server({ type: "something_new" });
    expect(events.map((e) => e.type)).toEqual(["resync", "notification"]);
  });

  it("reconnects with backoff after a drop, resubscribes and resyncs", () => {
    const { s, sockets, last, events, statuses } = setup();
    s.start();
    last().accept();
    s.subscribe("o:5");
    last().drop();
    expect(statuses.at(-1)).toBe("connecting");
    vi.advanceTimersByTime(backoffDelay(0, RECONNECT_BACKOFF, () => 0.5) - 1);
    expect(sockets).toHaveLength(1);
    vi.advanceTimersByTime(1);
    expect(sockets).toHaveLength(2);
    last().accept();
    expect(last().sent).toEqual([{ type: "subscribe", order_id: 5 }]);
    expect(events.filter((e) => e.type === "resync")).toHaveLength(2);
  });

  it("switches to polling after repeated failures and keeps retrying in the background", () => {
    const { s, sockets, last, statuses } = setup();
    s.start();
    for (let i = 0; i < POLL_AFTER_FAILURES; i++) {
      last().drop();
      vi.advanceTimersByTime(RECONNECT_BACKOFF.maxMs);
    }
    expect(statuses).toContain("polling");
    expect(sockets.length).toBeGreaterThan(POLL_AFTER_FAILURES);
    last().accept();
    expect(statuses.at(-1)).toBe("open");
  });

  it("waits longer after each failed attempt", () => {
    const { s, sockets, last } = setup();
    s.start();
    last().drop(); // 第 1 次失败：等 attempt 0
    vi.advanceTimersByTime(backoffDelay(0, RECONNECT_BACKOFF, () => 0.5));
    expect(sockets).toHaveLength(2);
    last().drop(); // 第 2 次失败：等 attempt 1，更久
    vi.advanceTimersByTime(backoffDelay(0, RECONNECT_BACKOFF, () => 0.5));
    expect(sockets).toHaveLength(2);
    vi.advanceTimersByTime(backoffDelay(1, RECONNECT_BACKOFF, () => 0.5));
    expect(sockets).toHaveLength(3);
  });

  it("abandons a half-open connection when pings go unanswered", () => {
    const { s, sockets, last } = setup();
    s.start();
    last().accept();
    vi.advanceTimersByTime(PING_INTERVAL_MS);
    expect(last().sent).toContainEqual({ type: "ping" });
    vi.advanceTimersByTime(PONG_TIMEOUT_MS);
    expect(must(sockets[0]).closed).toBe(true);
    vi.advanceTimersByTime(RECONNECT_BACKOFF.baseMs);
    expect(sockets).toHaveLength(2);
  });

  it("keeps the connection when the server answers pings", () => {
    const { s, sockets, last } = setup();
    s.start();
    last().accept();
    vi.advanceTimersByTime(PING_INTERVAL_MS);
    last().server({ type: "pong" });
    vi.advanceTimersByTime(PONG_TIMEOUT_MS);
    expect(must(sockets[0]).closed).toBe(false);
  });

  it("polls right away when the browser has no WebSocket", () => {
    const statuses: RealtimeStatus[] = [];
    const s = new RealtimeSocket({
      url: "ws://t",
      create: null,
      onEvent: () => {},
      onStatus: (st) => statuses.push(st),
    });
    s.start();
    expect(statuses).toEqual(["polling"]);
  });

  it("stops cleanly without reconnecting", () => {
    const { s, sockets, last } = setup();
    s.start();
    last().accept();
    s.stop();
    expect(must(sockets[0]).closed).toBe(true);
    vi.advanceTimersByTime(RECONNECT_BACKOFF.maxMs * 2);
    expect(sockets).toHaveLength(1);
  });

  it("only sends typing for subscribed topics and unsubscribes", () => {
    const { s, last } = setup();
    s.start();
    last().accept();
    s.typing("c:1");
    s.subscribe("c:1");
    s.typing("c:1");
    s.unsubscribe("c:1");
    expect(last().sent).toEqual([
      { type: "subscribe", conversation_id: 1 },
      { type: "typing", conversation_id: 1 },
      { type: "unsubscribe", conversation_id: 1 },
    ]);
  });
});
