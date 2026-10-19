import type { Schemas } from "@akari/api-client";

/** 报价卡片消息的事件 → 会话里的说明。 */
export const OFFER_CARD_EVENT: Record<string, string> = {
  created: "发出了担保报价",
  modified: "修改了担保报价",
  rejected: "拒绝了担保报价",
  cancelled: "取消了担保报价",
  paid: "买家已付款，担保订单已生成",
  payment_reverted: "付款被区块回滚撤销，报价恢复可付款",
};

/** 会话列表里最后一条消息的摘要。 */
export function messagePreview(m: Schemas["Message"], me: number): string {
  const mine = m.sender_id === me ? "我：" : "";
  switch (m.kind) {
    case "offer":
      return `[担保] ${OFFER_CARD_EVENT[m.body] ?? "报价有更新"}`;
    case "image":
      return `${mine}[图片]`;
    default:
      return `${mine}${m.body}`;
  }
}
