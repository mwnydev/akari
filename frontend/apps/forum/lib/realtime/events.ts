// WebSocket 推送的消息结构（协议见 forum-api `routes::ws` 模块文档）。
// 外层的 `type` 信封不在 OpenAPI 里；里面的消息、通知、在线状态都用生成的类型。
import type { Schemas } from "@akari/api-client";

export type Presence = Schemas["Presence"];

/** 订阅目标：私信会话或订单。 */
export type Topic = { conversation_id: number } | { order_id: number };

/** 服务端推送，以及页面之间转发的本地事件。 */
export type RealtimeEvent =
  | { type: "message"; message: Schemas["Message"] }
  | { type: "read"; conversation_id: number; user_id: number; last_id: number }
  | { type: "order_message"; message: Schemas["OrderMessage"] }
  | { type: "order_read"; order_id: number; user_id: number; last_id: number }
  | { type: "order"; order_id: number; state: string; level: string; correction: boolean }
  | { type: "notification"; notification: Schemas["Notification"] }
  | ({ type: "presence" } & Presence)
  | { type: "typing"; conversation_id?: number | null; order_id?: number | null; user_id: number }
  | {
      type: "subscribed";
      conversation_id?: number | null;
      order_id?: number | null;
      presence: Presence[];
    }
  /** 推送可能漏了（服务端广播断过，或本页刚重连）：按 REST 补拉。 */
  | { type: "resync" }
  /** 本地事件：别的页面把通知标成了已读。 */
  | { type: "notifications_read"; last_read_id: number; count: number };

export function topicKey(t: Topic): string {
  return "conversation_id" in t ? `c:${t.conversation_id}` : `o:${t.order_id}`;
}

export function topicOf(key: string): Topic | null {
  const m = /^([co]):(\d{1,18})$/.exec(key);
  if (!m) return null;
  const id = Number(m[2]);
  return m[1] === "c" ? { conversation_id: id } : { order_id: id };
}

/** 事件属于哪个会话 / 订单（用于按订阅过滤）；与具体会话无关的事件返回 null。 */
export function eventTopic(e: RealtimeEvent): string | null {
  switch (e.type) {
    case "message":
      return topicKey({ conversation_id: e.message.conversation_id });
    case "order_message":
      return topicKey({ order_id: e.message.order_id });
    case "read":
      return topicKey({ conversation_id: e.conversation_id });
    case "order_read":
    case "order":
      return topicKey({ order_id: e.order_id });
    case "typing":
    case "subscribed":
      if (typeof e.conversation_id === "number")
        return topicKey({ conversation_id: e.conversation_id });
      if (typeof e.order_id === "number") return topicKey({ order_id: e.order_id });
      return null;
    default:
      return null;
  }
}

const isObject = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null;
const isInt = (v: unknown): v is number => Number.isSafeInteger(v);

/**
 * 解析一条服务端推送；不认识或结构不对的返回 null（协议加新事件时旧页面直接忽略）。
 * `ready`、`pong`、`error`、`unsubscribed` 是连接内部的回复，由 RealtimeSocket 自己处理。
 */
export function parseEvent(raw: unknown): RealtimeEvent | null {
  let v: unknown;
  try {
    v = typeof raw === "string" ? JSON.parse(raw) : raw;
  } catch {
    return null;
  }
  if (!isObject(v) || typeof v.type !== "string") return null;
  switch (v.type) {
    case "message":
      return isObject(v.message) && isInt(v.message.id) && isInt(v.message.conversation_id)
        ? (v as RealtimeEvent)
        : null;
    case "order_message":
      return isObject(v.message) && isInt(v.message.id) && isInt(v.message.order_id)
        ? (v as RealtimeEvent)
        : null;
    case "read":
      return isInt(v.conversation_id) && isInt(v.user_id) && isInt(v.last_id)
        ? (v as RealtimeEvent)
        : null;
    case "order_read":
      return isInt(v.order_id) && isInt(v.user_id) && isInt(v.last_id)
        ? (v as RealtimeEvent)
        : null;
    case "order":
      return isInt(v.order_id) && typeof v.state === "string" ? (v as RealtimeEvent) : null;
    case "notification":
      return isObject(v.notification) && isInt(v.notification.id) ? (v as RealtimeEvent) : null;
    case "presence":
      return isInt(v.user_id) && typeof v.online === "boolean" ? (v as RealtimeEvent) : null;
    case "typing":
      return isInt(v.user_id) ? (v as RealtimeEvent) : null;
    case "subscribed":
      return Array.isArray(v.presence) ? (v as RealtimeEvent) : null;
    case "resync":
      return { type: "resync" };
    case "notifications_read":
      return isInt(v.last_read_id) && isInt(v.count) ? (v as RealtimeEvent) : null;
    default:
      return null;
  }
}
