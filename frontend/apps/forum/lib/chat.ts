// 私信、订单聊天与通知列表的合并规则：REST 分页、重连补拉与 WebSocket 推送可能重复或乱序到达，一律按 ID 去重排序。
import type { Schemas } from "@akari/api-client";
import { UNREAD_CAP } from "./badge";

type Conversation = Schemas["ConversationView"];
type Message = Schemas["Message"];

/** 合并两段消息：按 ID 去重（后来的覆盖先到的）、按 ID 升序（即时间正序）。 */
export function mergeById<T extends { id: number }>(current: readonly T[], incoming: readonly T[]) {
  if (!incoming.length) return current as T[];
  const byId = new Map<number, T>();
  for (const m of current) byId.set(m.id, m);
  for (const m of incoming) byId.set(m.id, m);
  return [...byId.values()].sort((a, b) => a.id - b.id);
}

/** 通知列表新的在前：按 ID 去重、降序。 */
export function mergeNewestFirst<T extends { id: number }>(
  current: readonly T[],
  incoming: readonly T[],
) {
  return mergeById(current, incoming).reverse();
}

/** 列表里最大的 ID（补拉用的 `after` 游标）；空列表为 0。 */
export function lastId(list: readonly { id: number }[]): number {
  return list.reduce((max, m) => Math.max(max, m.id), 0);
}

/**
 * 收到一条私信后更新会话列表：该会话移到最前、换上最后一条消息；对方发来且我没在看这个会话时未读加一。
 * 列表里没有这个会话（新会话，或在后面的页里）返回 null，由调用方重新拉取第一页。
 */
export function applyMessage(
  list: readonly Conversation[],
  msg: Message,
  me: number,
  viewing: number | null,
): Conversation[] | null {
  const i = list.findIndex((c) => c.id === msg.conversation_id);
  const c = list[i];
  if (!c) return null;
  if (c.last_message && c.last_message.id >= msg.id) return list as Conversation[];
  const fromPeer = msg.sender_id !== me;
  const updated: Conversation = {
    ...c,
    last_message: msg,
    unread:
      fromPeer && viewing !== c.id && msg.id > c.my_read_id
        ? Math.min(UNREAD_CAP, c.unread + 1)
        : c.unread,
  };
  return [updated, ...list.slice(0, i), ...list.slice(i + 1)];
}

/** 已读回执：我自己（别的设备）读了 → 未读清零；对方读了 → 更新回执位置。已读位置只前进。 */
export function applyRead(
  list: readonly Conversation[],
  ev: { conversation_id: number; user_id: number; last_id: number },
  me: number,
): Conversation[] {
  return list.map((c) => {
    if (c.id !== ev.conversation_id) return c;
    if (ev.user_id === me) {
      if (ev.last_id <= c.my_read_id) return c;
      const caughtUp = !c.last_message || ev.last_id >= c.last_message.id;
      return { ...c, my_read_id: ev.last_id, unread: caughtUp ? 0 : c.unread };
    }
    return ev.last_id > c.peer_read_id ? { ...c, peer_read_id: ev.last_id } : c;
  });
}

/** 我在看这个会话：应当上报的已读位置（对方发来的最新一条）；不需要上报返回 null。 */
export function readTarget(messages: readonly Message[], me: number, myReadId: number) {
  let target = 0;
  for (const m of messages) if (m.sender_id !== me && m.id > target) target = m.id;
  return target > myReadId ? target : null;
}

/**
 * 订单聊天哈希链的本地检查：已加载的连续消息里，每条的 `prev_hash` 应等于上一条的 `hash`。
 * 返回第一条接不上的消息 ID；完整返回 null。整条链的重算以后端 verify 接口为准。
 */
export function chainBreak(messages: readonly { id: number; prev_hash: string; hash: string }[]) {
  for (let i = 1; i < messages.length; i++) {
    const prev = messages[i - 1];
    const cur = messages[i];
    if (prev && cur && cur.prev_hash.toLowerCase() !== prev.hash.toLowerCase()) return cur.id;
  }
  return null;
}
