// 通知的显示文字与跳转地址。`payload` 的结构按 `kind` 而定（后端 `forum_core::chat::Notification`），
// 是任意 JSON，这里逐个字段检查类型，缺了就退回不带链接的通用说法。
import type { Schemas } from "@akari/api-client";
import { topicHref } from "./links";
import { ORDER_STATE } from "./orders";
import { floorPage } from "./pagination";

export type Notification = Schemas["Notification"];

export interface NotificationView {
  text: string;
  href: string | null;
}

const int = (v: unknown): number | null => (Number.isSafeInteger(v) ? (v as number) : null);
const str = (v: unknown): string | null => (typeof v === "string" ? v : null);

const OFFER_EVENT: Record<string, string> = {
  created: "发来一份担保报价",
  modified: "修改了担保报价",
  rejected: "拒绝了你的担保报价",
  cancelled: "取消了担保报价",
  paid: "已付款，担保订单已生成",
  payment_reverted: "的付款因区块回滚被撤销",
};

/** 通知 → 一句中文和跳转地址。`actor` 是触发者的显示名（系统通知为 null）。 */
export function describeNotification(n: Notification, actor: string | null): NotificationView {
  const p = n.payload as Record<string, unknown>;
  const who = actor ?? "有人";
  switch (n.kind) {
    case "reply":
    case "mention": {
      const topic = int(p.topic_id);
      const floor = int(p.floor);
      const text =
        n.kind === "reply"
          ? `${who} 回复了你${floor ? `（#${floor} 楼）` : ""}`
          : `${who} 在${floor ? ` #${floor} 楼` : "帖子里"}提到了你`;
      return {
        text,
        href: topic ? topicHref(topic, floor ? floorPage(floor) : 1, floor ?? undefined) : null,
      };
    }
    case "message": {
      const c = int(p.conversation_id);
      return { text: `${who} 给你发了私信`, href: c ? `/messages/${c}` : null };
    }
    case "offer": {
      const c = int(p.conversation_id);
      const event = str(p.event);
      const what = (event && OFFER_EVENT[event]) ?? "更新了担保报价";
      return { text: `${who} ${what}`, href: c ? `/messages/${c}` : null };
    }
    case "order": {
      const id = int(p.order_id);
      const state = str(p.state);
      const label = (state && ORDER_STATE[state]) ?? "状态有更新";
      const correction = p.correction === true ? "（区块回滚后的状态修正）" : "";
      return {
        text: `订单${id ? ` #${id}` : ""} ${label}${correction}`,
        href: id ? `/orders/${id}` : null,
      };
    }
    case "system":
      return describeSystem(p);
    default:
      return { text: "系统通知", href: null };
  }
}

/** 系统通知：按 `event` 区分（后端 `notify.<event>`）；不认识的只显示「系统通知」。 */
function describeSystem(p: Record<string, unknown>): NotificationView {
  if (p.event === "report_resolved") {
    // 只说处理结果，不含处理方式与处罚内容（后端 report_case_resolve.sql）
    const topic = int(p.topic_id);
    const floor = int(p.floor);
    const where = floor ? `（#${floor} 楼）` : "";
    return {
      text:
        p.outcome === "actioned"
          ? `你举报的内容${where}已处理，感谢反馈`
          : `你举报的内容${where}经核查未发现违规`,
      href: topic ? topicHref(topic, floor ? floorPage(floor) : 1, floor ?? undefined) : null,
    };
  }
  return { text: "系统通知", href: null };
}
