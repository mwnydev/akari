import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { RealtimeEvent } from "./events";
import { RealtimeHub, socketUrl, type ChannelLike, type LocksLike, type Wire } from "./hub";
import type { SocketLike } from "./socket";

function must<T>(v: T | null | undefined): T {
  if (v == null) throw new Error("missing");
  return v;
}

/** 同步投递的 BroadcastChannel：发送方自己收不到，与浏览器一致。 */
class Bus {
  members = new Set<FakeChannel>();
  open = () => new FakeChannel(this);
}
class FakeChannel implements ChannelLike {
  onmessage: ChannelLike["onmessage"] = null;
  constructor(private bus: Bus) {
    bus.members.add(this);
  }
  postMessage(msg: Wire) {
    for (const m of this.bus.members) if (m !== this) m.onmessage?.({ data: structuredClone(msg) });
  }
  close() {
    this.bus.members.delete(this);
  }
}

/** 单个锁的排队实现（够模拟 Web Locks 的独占与 AbortSignal 撤回）。 */
class FakeLocks implements LocksLike {
  private held = false;
  private queue: { cb: () => Promise<void>; resolve: () => void; signal: AbortSignal }[] = [];
  request(_name: string, opts: { signal: AbortSignal }, cb: () => Promise<void>) {
    return new Promise<unknown>((resolve, reject) => {
      opts.signal.addEventListener("abort", () => {
        this.queue = this.queue.filter((q) => q.cb !== cb);
        reject(new DOMException("aborted", "AbortError"));
      });
      this.queue.push({ cb, resolve: () => resolve(undefined), signal: opts.signal });
      this.pump();
    });
  }
  private pump() {
    if (this.held) return;
    const next = this.queue.shift();
    if (!next) return;
    this.held = true;
    void next.cb().then(() => {
      this.held = false;
      next.resolve();
      this.pump();
    });
  }
}

class FakeSocket implements SocketLike {
  readyState = 0;
  sent: unknown[] = [];
  onopen: SocketLike["onopen"] = null;
  onmessage: SocketLike["onmessage"] = null;
  onclose: SocketLike["onclose"] = null;
  onerror: SocketLike["onerror"] = null;
  send(d: string) {
    this.sent.push(JSON.parse(d));
  }
  close() {
    this.readyState = 3;
  }
  accept() {
    this.readyState = 1;
    this.server({ type: "ready", user_id: 1, heartbeat_secs: 30 });
  }
  server(msg: object) {
    this.onmessage?.({ data: JSON.stringify(msg) });
  }
}

function world() {
  const bus = new Bus();
  const locks = new FakeLocks();
  const sockets: FakeSocket[] = [];
  const tab = () => {
    const events: RealtimeEvent[] = [];
    const hub = new RealtimeHub({
      url: "ws://t/api/v1/ws",
      createSocket: () => {
        const s = new FakeSocket();
        sockets.push(s);
        return s;
      },
      openChannel: bus.open,
      locks,
    });
    hub.on((e) => events.push(e));
    return { hub, events };
  };
  return { tab, sockets };
}

const flush = () => new Promise((r) => setTimeout(r, 0));

describe("RealtimeHub across tabs", () => {
  beforeEach(() => vi.useRealTimers());
  afterEach(() => vi.useRealTimers());

  it("shares one connection: followers' subscriptions go through the leader", async () => {
    const { tab, sockets } = world();
    const a = tab();
    const b = tab();
    a.hub.start();
    await flush();
    b.hub.start();
    await flush();
    expect(a.hub.isLeader).toBe(true);
    expect(b.hub.isLeader).toBe(false);
    expect(sockets).toHaveLength(1);
    must(sockets[0]).accept();

    const off = b.hub.subscribe({ conversation_id: 9 });
    b.hub.subscribe({ conversation_id: 9 }); // 同页第二个订阅者不重复发
    expect(must(sockets[0]).sent).toEqual([{ type: "subscribe", conversation_id: 9 }]);
    b.hub.typing({ conversation_id: 9 });
    expect(must(sockets[0]).sent.at(-1)).toEqual({ type: "typing", conversation_id: 9 });

    must(sockets[0]).server({ type: "message", message: { id: 1, conversation_id: 9 } });
    expect(a.events.map((e) => e.type)).toContain("message");
    expect(b.events.map((e) => e.type)).toContain("message");
    expect(b.hub.status).toBe("open");

    off();
    expect(must(sockets[0]).sent.at(-1)).toEqual({ type: "typing", conversation_id: 9 });
  });

  it("filters typing / subscribed events to tabs that opened that conversation", async () => {
    const { tab, sockets } = world();
    const a = tab();
    const b = tab();
    a.hub.start();
    await flush();
    b.hub.start();
    await flush();
    must(sockets[0]).accept();
    b.hub.subscribe({ order_id: 4 });
    must(sockets[0]).server({ type: "typing", order_id: 4, user_id: 2 });
    expect(b.events.some((e) => e.type === "typing")).toBe(true);
    expect(a.events.some((e) => e.type === "typing")).toBe(false);
  });

  it("hands the connection to another tab when the leader closes, keeping subscriptions", async () => {
    const { tab, sockets } = world();
    const a = tab();
    const b = tab();
    a.hub.start();
    await flush();
    b.hub.start();
    await flush();
    b.hub.subscribe({ conversation_id: 3 });
    a.hub.stop();
    await flush();
    expect(b.hub.isLeader).toBe(true);
    expect(sockets).toHaveLength(2);
    must(sockets[1]).accept();
    expect(must(sockets[1]).sent).toEqual([{ type: "subscribe", conversation_id: 3 }]);
    expect(b.events.map((e) => e.type)).toContain("resync");
  });

  it("drops a closed follower's subscriptions from the shared connection", async () => {
    const { tab, sockets } = world();
    const a = tab();
    const b = tab();
    a.hub.start();
    await flush();
    b.hub.start();
    await flush();
    must(sockets[0]).accept();
    b.hub.subscribe({ order_id: 8 });
    b.hub.stop();
    expect(must(sockets[0]).sent.at(-1)).toEqual({ type: "unsubscribe", order_id: 8 });
  });

  it("broadcasts local events to every tab", async () => {
    const { tab } = world();
    const a = tab();
    const b = tab();
    a.hub.start();
    b.hub.start();
    await flush();
    b.hub.broadcast({ type: "notifications_read", last_read_id: 5, count: 0 });
    expect(a.events).toContainEqual({ type: "notifications_read", last_read_id: 5, count: 0 });
    expect(b.events).toContainEqual({ type: "notifications_read", last_read_id: 5, count: 0 });
  });

  it("falls back to its own connection without BroadcastChannel / Web Locks", () => {
    const sockets: FakeSocket[] = [];
    const hub = new RealtimeHub({
      url: "ws://t",
      createSocket: () => {
        const s = new FakeSocket();
        sockets.push(s);
        return s;
      },
      openChannel: null,
      locks: null,
    });
    hub.start();
    expect(hub.isLeader).toBe(true);
    expect(sockets).toHaveLength(1);
  });
});

describe("socketUrl", () => {
  it("follows the page scheme and host", () => {
    expect(socketUrl({ protocol: "https:", host: "akari.cc" })).toBe("wss://akari.cc/api/v1/ws");
    expect(socketUrl({ protocol: "http:", host: "localhost:3000" })).toBe(
      "ws://localhost:3000/api/v1/ws",
    );
  });
});
