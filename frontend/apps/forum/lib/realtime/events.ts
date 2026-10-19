// WebSocket 推送的消息结构（协议见 forum-api `routes::ws` 模块文档）。
// 外层的 `type` 信封不在 OpenAPI 里；里面的消息、通知、在线状态都用生成的类型。
import type { Schemas } from "@akari/api-client";
import type { AuctionState } from "../auction";

export type Presence = Schemas["Presence"];

/** 订阅目标：私信会话、订单或主题页（楼层评论与点赞）。 */
export type Topic = { conversation_id: number } | { order_id: number } | { topic_id: number };

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
      topic_id?: number | null;
      presence: Presence[];
    }
  /** 订阅的主题页：楼层下新增（或恢复）一条评论；`comments` 是该楼现在的评论数 */
  | { type: "comment"; topic_id: number; comment: Schemas["FloorComment"]; comments: number }
  | {
      type: "comment_removed";
      topic_id: number;
      post_id: number;
      comment_id: number;
      comments: number;
    }
  /** 订阅的主题页：`user_id` 赞了或取消赞，`likes` 是该楼现在的点赞数 */
  | {
      type: "likes";
      topic_id: number;
      post_id: number;
      user_id: number;
      liked: boolean;
      likes: number;
    }
  /**
   * 订阅的拍卖帖：新出价（金额是拍卖币种最小单位，出价人已打码；`linked` 表示与本场其他出价人同设备，
   * `ends_at` 是防狙击顺延后的结束时间）
   */
  | {
      type: "auction_bid";
      topic_id: number;
      auction_id: number;
      bid_id: number;
      amount: string;
      bidder: string;
      linked: boolean;
      bids: number;
      ends_at: string;
      reserve_met: boolean;
    }
  /** 订阅的拍卖帖：状态变了（结束、成交待付款、已成交、流拍、取消），详情按 REST 重取 */
  | { type: "auction_state"; topic_id: number; auction_id: number; state: AuctionState }
  /** 推送可能漏了（服务端广播断过，或本页刚重连）：按 REST 补拉。 */
  | { type: "resync" }
  /** 本地事件：别的页面把通知标成了已读。 */
  | { type: "notifications_read"; last_read_id: number; count: number };

export function topicKey(t: Topic): string {
  if ("conversation_id" in t) return `c:${t.conversation_id}`;
  if ("order_id" in t) return `o:${t.order_id}`;
  return `t:${t.topic_id}`;
}

export function topicOf(key: string): Topic | null {
  const m = /^([cot]):(\d{1,18})$/.exec(key);
  if (!m) return null;
  const id = Number(m[2]);
  if (m[1] === "c") return { conversation_id: id };
  return m[1] === "o" ? { order_id: id } : { topic_id: id };
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
      if (e.type === "subscribed" && typeof e.topic_id === "number")
        return topicKey({ topic_id: e.topic_id });
      return null;
    case "comment":
    case "comment_removed":
    case "likes":
    case "auction_bid":
    case "auction_state":
      return topicKey({ topic_id: e.topic_id });
    default:
      return null;
  }
}

const isObject = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null;
const isInt = (v: unknown): v is number => Number.isSafeInteger(v);
const isDecimal = (v: unknown): v is string => typeof v === "string" && /^\d{1,78}$/.test(v);
const AUCTION_STATES: ReadonlySet<string> = new Set<AuctionState>([
  "draft",
  "live",
  "ended",
  "awaiting_payment",
  "settled",
  "unsold",
  "cancelled",
]);
const isTime = (v: unknown): v is string => typeof v === "string" && !Number.isNaN(Date.parse(v));

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
    case "comment":
      return isInt(v.topic_id) &&
        isInt(v.comments) &&
        isObject(v.comment) &&
        isInt(v.comment.id) &&
        isInt(v.comment.post_id) &&
        isInt(v.comment.author_id) &&
        typeof v.comment.body === "string"
        ? (v as RealtimeEvent)
        : null;
    case "comment_removed":
      return isInt(v.topic_id) && isInt(v.post_id) && isInt(v.comment_id) && isInt(v.comments)
        ? (v as RealtimeEvent)
        : null;
    case "likes":
      return isInt(v.topic_id) &&
        isInt(v.post_id) &&
        isInt(v.user_id) &&
        typeof v.liked === "boolean" &&
        isInt(v.likes)
        ? (v as RealtimeEvent)
        : null;
    case "auction_bid":
      return isInt(v.topic_id) &&
        isInt(v.auction_id) &&
        isInt(v.bid_id) &&
        isDecimal(v.amount) &&
        typeof v.bidder === "string" &&
        typeof v.linked === "boolean" &&
        isInt(v.bids) &&
        typeof v.reserve_met === "boolean" &&
        isTime(v.ends_at)
        ? (v as RealtimeEvent)
        : null;
    case "auction_state":
      return isInt(v.topic_id) &&
        isInt(v.auction_id) &&
        typeof v.state === "string" &&
        AUCTION_STATES.has(v.state)
        ? (v as RealtimeEvent)
        : null;
    case "resync":
      return { type: "resync" };
    case "notifications_read":
      return isInt(v.last_read_id) && isInt(v.count) ? (v as RealtimeEvent) : null;
    default:
      return null;
  }
}
