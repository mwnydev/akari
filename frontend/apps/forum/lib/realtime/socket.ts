// 一条到 forum-api 的 WebSocket 连接：断线按退避加抖动重连、重连后重新订阅并通知页面补拉、
// 应用层心跳发现「假在线」、连续失败后让页面改为轮询（连接仍在后台继续尝试）。
import { RECONNECT_BACKOFF, backoffDelay, type Backoff } from "./backoff";
import { parseEvent, topicOf, type RealtimeEvent } from "./events";

/**
 * `connecting`：正在连接（首次或刚断开）；`open`：推送可用；
 * `polling`：连续多次连不上（或浏览器不支持），页面应定时用 REST 拉取。
 */
export type RealtimeStatus = "connecting" | "open" | "polling";

/** 用到的 WebSocket 子集（测试里换成假的）。 */
export interface SocketLike {
  readonly readyState: number;
  send(data: string): void;
  close(code?: number, reason?: string): void;
  onopen: ((ev: unknown) => void) | null;
  onmessage: ((ev: { data: unknown }) => void) | null;
  onclose: ((ev: unknown) => void) | null;
  onerror: ((ev: unknown) => void) | null;
}

export interface SocketOptions {
  url: string;
  /** null = 浏览器不支持 WebSocket，直接进入轮询 */
  create: ((url: string) => SocketLike) | null;
  onEvent: (e: RealtimeEvent) => void;
  onStatus: (s: RealtimeStatus) => void;
  backoff?: Backoff | undefined;
  random?: (() => number) | undefined;
}

const OPEN = 1;
/** 应用层心跳间隔：比服务端 90 秒空闲断开短得多，也用来发现半开连接 */
export const PING_INTERVAL_MS = 25_000;
/** 发出 ping 后这么久没收到任何消息，当作连接已死 */
export const PONG_TIMEOUT_MS = 10_000;
/** 连续这么多次没连上就让页面改为轮询 */
export const POLL_AFTER_FAILURES = 3;

export class RealtimeSocket {
  private ws: SocketLike | null = null;
  private readonly topics = new Set<string>();
  private failures = 0;
  private stopped = true;
  private status: RealtimeStatus | null = null;
  private retryTimer: ReturnType<typeof setTimeout> | undefined;
  private pingTimer: ReturnType<typeof setInterval> | undefined;
  private pongTimer: ReturnType<typeof setTimeout> | undefined;

  constructor(private readonly opts: SocketOptions) {}

  start() {
    if (!this.stopped) return;
    this.stopped = false;
    if (!this.opts.create) {
      this.setStatus("polling");
      return;
    }
    this.connect();
  }

  stop() {
    this.stopped = true;
    clearTimeout(this.retryTimer);
    this.clearHeartbeat();
    const ws = this.ws;
    this.ws = null;
    if (ws) {
      ws.onclose = ws.onmessage = ws.onerror = ws.onopen = null;
      ws.close(1000, "page closed");
    }
  }

  /** 网络恢复、页面回到前台：等待中的重连立即进行。 */
  retryNow() {
    if (this.stopped || this.ws || !this.opts.create) return;
    clearTimeout(this.retryTimer);
    this.connect();
  }

  subscribe(key: string) {
    if (this.topics.has(key)) return;
    this.topics.add(key);
    this.sendTopic("subscribe", key);
  }

  unsubscribe(key: string) {
    if (!this.topics.delete(key)) return;
    this.sendTopic("unsubscribe", key);
  }

  typing(key: string) {
    if (this.topics.has(key)) this.sendTopic("typing", key);
  }

  private sendTopic(type: "subscribe" | "unsubscribe" | "typing", key: string) {
    const topic = topicOf(key);
    if (topic) this.send({ type, ...topic });
  }

  private send(msg: object): boolean {
    if (this.ws?.readyState !== OPEN) return false;
    this.ws.send(JSON.stringify(msg));
    return true;
  }

  private setStatus(s: RealtimeStatus) {
    if (s === this.status) return;
    this.status = s;
    this.opts.onStatus(s);
  }

  private connect() {
    const create = this.opts.create;
    if (!create) return;
    if (this.status !== "polling") this.setStatus("connecting");
    let ws: SocketLike;
    try {
      ws = create(this.opts.url);
    } catch {
      this.failures++;
      this.scheduleRetry();
      return;
    }
    this.ws = ws;
    let ready = false;
    ws.onmessage = (ev) => {
      this.alive();
      let msg: unknown;
      try {
        msg = typeof ev.data === "string" ? JSON.parse(ev.data) : null;
      } catch {
        return;
      }
      const type = typeof msg === "object" && msg !== null && "type" in msg ? msg.type : null;
      if (type === "ready") {
        ready = true;
        this.onReady();
        return;
      }
      if (type === "pong" || type === "error" || type === "unsubscribed") return;
      const event = parseEvent(msg);
      if (event) this.opts.onEvent(event);
    };
    ws.onclose = () => this.dropped(ws, ready);
    ws.onerror = () => {
      // 随后必有 onclose，在那里处理
    };
  }

  private onReady() {
    this.failures = 0;
    for (const key of this.topics) this.sendTopic("subscribe", key);
    this.startHeartbeat();
    this.setStatus("open");
    // 连上之前（含断线期间）的推送都收不到，页面按 REST 补拉一次
    this.opts.onEvent({ type: "resync" });
  }

  /** 连接断了（或心跳超时被放弃）：解除回调，安排重连。 */
  private dropped(ws: SocketLike, wasReady: boolean) {
    if (this.ws !== ws) return;
    this.ws = null;
    ws.onclose = ws.onmessage = ws.onerror = ws.onopen = null;
    this.clearHeartbeat();
    // 连上过（收到 ready）再断开的从头计数；握手都没完成的算一次失败
    this.failures = wasReady ? 0 : this.failures + 1;
    this.scheduleRetry();
  }

  private scheduleRetry() {
    if (this.stopped) return;
    this.setStatus(this.failures >= POLL_AFTER_FAILURES ? "polling" : "connecting");
    const delay = backoffDelay(Math.max(0, this.failures - 1), this.backoff(), this.opts.random);
    clearTimeout(this.retryTimer);
    this.retryTimer = setTimeout(() => this.connect(), delay);
  }

  private backoff(): Backoff {
    return this.opts.backoff ?? RECONNECT_BACKOFF;
  }

  private startHeartbeat() {
    this.clearHeartbeat();
    this.pingTimer = setInterval(() => {
      if (!this.send({ type: "ping" })) return;
      clearTimeout(this.pongTimer);
      this.pongTimer = setTimeout(() => {
        // 半开的连接 close() 可能很久才触发 onclose：直接放弃它
        const ws = this.ws;
        if (!ws) return;
        this.dropped(ws, true);
        ws.close(4000, "pong timeout");
      }, PONG_TIMEOUT_MS);
    }, PING_INTERVAL_MS);
  }

  /** 收到任何消息都说明连接活着。 */
  private alive() {
    clearTimeout(this.pongTimer);
  }

  private clearHeartbeat() {
    clearInterval(this.pingTimer);
    clearTimeout(this.pongTimer);
  }
}
