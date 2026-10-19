// 同一浏览器的多个标签页共用一条 WebSocket（服务端每人最多 5 条连接）：
// 用 Web Locks 选出一个「主页面」持有连接，其他页面经 BroadcastChannel 把订阅交给它、从它那里收事件。
// 浏览器缺任何一个 API 时退回每个页面各自一条连接。
import { eventTopic, topicKey, type RealtimeEvent, type Topic } from "./events";
import { RealtimeSocket, type RealtimeStatus, type SocketLike } from "./socket";

/** 页面之间的消息。`tab` 是发送方页面的随机 ID。 */
export type Wire =
  | { t: "event"; e: RealtimeEvent }
  | { t: "status"; s: RealtimeStatus }
  /** 新的主页面就位：其他页面重新报告订阅 */
  | { t: "leader" }
  /** 页面报告自己的全部订阅（启动时、换了主页面后） */
  | { t: "hello"; tab: string; keys: string[] }
  | { t: "sub" | "unsub"; tab: string; key: string }
  | { t: "typing"; key: string }
  | { t: "bye"; tab: string };

export interface ChannelLike {
  postMessage(msg: Wire): void;
  onmessage: ((ev: { data: Wire }) => void) | null;
  close(): void;
}

export interface LocksLike {
  request(
    name: string,
    options: { signal: AbortSignal },
    callback: () => Promise<void>,
  ): Promise<unknown>;
}

export interface HubEnv {
  url: string;
  createSocket: ((url: string) => SocketLike) | null;
  /** 两者都有才在页面之间共用连接；频道每次 start() 新开、stop() 关闭 */
  openChannel: (() => ChannelLike) | null;
  locks: LocksLike | null;
  random?: (() => number) | undefined;
}

const LOCK_NAME = "akari-realtime";

export class RealtimeHub {
  status: RealtimeStatus = "connecting";
  readonly tab: string;
  private readonly listeners = new Set<(e: RealtimeEvent) => void>();
  private readonly statusListeners = new Set<(s: RealtimeStatus) => void>();
  /** 本页面的订阅（引用计数：同一会话可能被多个组件订阅） */
  private readonly local = new Map<string, number>();
  /** 主页面才有：各页面的订阅与真正的连接 */
  private readonly byTab = new Map<string, Set<string>>();
  private socket: RealtimeSocket | null = null;
  private socketKeys = new Set<string>();
  private releaseLock: (() => void) | null = null;
  private channel: ChannelLike | null = null;
  /** 还在排队等锁时 stop() 用它撤回请求 */
  private lockWait: AbortController | null = null;
  private started = false;

  constructor(private readonly env: HubEnv) {
    this.tab = Math.floor((env.random ?? Math.random)() * 2 ** 48).toString(36);
  }

  get isLeader(): boolean {
    return this.socket !== null;
  }

  start() {
    if (this.started) return;
    this.started = true;
    const { openChannel, locks } = this.env;
    if (!openChannel || !locks) {
      this.lead();
      return;
    }
    const channel = (this.channel = openChannel());
    channel.onmessage = (ev) => this.onWire(ev.data);
    const wait = (this.lockWait = new AbortController());
    locks
      .request(LOCK_NAME, { signal: wait.signal }, () => {
        this.lockWait = null;
        this.lead();
        // 持有锁直到 stop()（或页面关闭时浏览器自动释放）
        return new Promise<void>((resolve) => (this.releaseLock = resolve));
      })
      .catch(() => {
        // stop() 撤回了排队中的请求
      });
    channel.postMessage({ t: "hello", tab: this.tab, keys: [...this.local.keys()] });
  }

  stop() {
    if (!this.started) return;
    this.started = false;
    this.socket?.stop();
    this.socket = null;
    this.socketKeys.clear();
    this.byTab.clear();
    this.lockWait?.abort();
    this.lockWait = null;
    this.releaseLock?.();
    this.releaseLock = null;
    if (this.channel) {
      this.channel.postMessage({ t: "bye", tab: this.tab });
      this.channel.onmessage = null;
      this.channel.close();
      this.channel = null;
    }
  }

  /** 网络恢复或页面回到前台时调用：主页面立即重连。 */
  retryNow() {
    this.socket?.retryNow();
  }

  on(fn: (e: RealtimeEvent) => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  onStatus(fn: (s: RealtimeStatus) => void): () => void {
    this.statusListeners.add(fn);
    return () => this.statusListeners.delete(fn);
  }

  /** 订阅一个会话 / 订单（收对方上下线、正在输入；仲裁员靠它收订单推送）。返回取消订阅。 */
  subscribe(topic: Topic): () => void {
    const key = topicKey(topic);
    const n = this.local.get(key) ?? 0;
    this.local.set(key, n + 1);
    if (n === 0) this.changeOwn("sub", key);
    let done = false;
    return () => {
      if (done) return;
      done = true;
      const left = (this.local.get(key) ?? 1) - 1;
      if (left > 0) {
        this.local.set(key, left);
        return;
      }
      this.local.delete(key);
      this.changeOwn("unsub", key);
    };
  }

  typing(topic: Topic) {
    const key = topicKey(topic);
    if (this.socket) this.socket.typing(key);
    else this.channel?.postMessage({ t: "typing", key });
  }

  /** 把本地事件发给所有页面（含本页），如「通知已读」。 */
  broadcast(e: RealtimeEvent) {
    this.deliver(e);
    this.channel?.postMessage({ t: "event", e });
  }

  private changeOwn(op: "sub" | "unsub", key: string) {
    if (this.socket) {
      this.apply(op, this.tab, key);
    } else if (this.started) {
      this.channel?.postMessage({ t: op, tab: this.tab, key });
    }
  }

  private lead() {
    this.byTab.set(this.tab, new Set(this.local.keys()));
    this.socket = new RealtimeSocket({
      url: this.env.url,
      create: this.env.createSocket,
      random: this.env.random,
      onEvent: (e) => {
        this.deliver(e);
        this.channel?.postMessage({ t: "event", e });
      },
      onStatus: (s) => {
        this.setStatus(s);
        this.channel?.postMessage({ t: "status", s });
      },
    });
    this.sync();
    this.socket.start();
    this.channel?.postMessage({ t: "leader" });
  }

  private onWire(msg: Wire) {
    if (!this.started) return;
    if (this.socket) {
      switch (msg.t) {
        case "hello":
          this.byTab.set(msg.tab, new Set(msg.keys));
          this.sync();
          this.channel?.postMessage({ t: "status", s: this.status });
          return;
        case "sub":
        case "unsub":
          this.apply(msg.t, msg.tab, msg.key);
          return;
        case "typing":
          this.socket.typing(msg.key);
          return;
        case "bye":
          this.byTab.delete(msg.tab);
          this.sync();
          return;
        case "event":
          // 别的页面广播的本地事件
          this.deliver(msg.e);
          return;
        default:
          return;
      }
    }
    switch (msg.t) {
      case "event":
        this.deliver(msg.e);
        return;
      case "status":
        this.setStatus(msg.s);
        return;
      case "leader":
        this.channel?.postMessage({ t: "hello", tab: this.tab, keys: [...this.local.keys()] });
        return;
      default:
        return;
    }
  }

  private apply(op: "sub" | "unsub", tab: string, key: string) {
    let keys = this.byTab.get(tab);
    if (!keys) this.byTab.set(tab, (keys = new Set()));
    if (op === "sub") keys.add(key);
    else keys.delete(key);
    this.sync();
  }

  /** 让连接上的订阅等于所有页面订阅的并集。 */
  private sync() {
    const socket = this.socket;
    if (!socket) return;
    const want = new Set<string>();
    for (const keys of this.byTab.values()) for (const k of keys) want.add(k);
    for (const k of want) if (!this.socketKeys.has(k)) socket.subscribe(k);
    for (const k of this.socketKeys) if (!want.has(k)) socket.unsubscribe(k);
    this.socketKeys = want;
  }

  private deliver(e: RealtimeEvent) {
    // 只与某个会话 / 订单有关的状态类事件，本页面没订阅就不必处理
    const key = e.type === "typing" || e.type === "subscribed" ? eventTopic(e) : null;
    if (key !== null && !this.local.has(key)) return;
    for (const fn of this.listeners) fn(e);
  }

  private setStatus(s: RealtimeStatus) {
    if (s === this.status) return;
    this.status = s;
    for (const fn of this.statusListeners) fn(s);
  }
}

/** 浏览器里的连接地址：与页面同源（生产由 Caddy、开发由 Next rewrite 转给 forum-api）。 */
export function socketUrl(loc: { protocol: string; host: string }): string {
  return `${loc.protocol === "https:" ? "wss" : "ws"}://${loc.host}/api/v1/ws`;
}
