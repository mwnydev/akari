import type { BadgeTone, OnceState } from "@akari/ui";
import type { OrderView } from "./api";

// 订单页的展示与按钮规则（纯函数，不依赖钱包 SDK，可以进首屏 JS）。数据来自 indexer（GET /api/v1/orders/{id}）。

/** 一次性担保的进度；`none`（回滚后订单消失）与不认识的状态返回 null。确认期限有值 = 卖家已标记发货。 */
export function onceState(o: OrderView): OnceState | null {
  switch (o.state) {
    case "funded":
      return o.deadlines.confirm ? "delivered" : "funded";
    case "disputed":
    case "released":
    case "refunded":
    case "settled":
      return o.state;
    default:
      return null;
  }
}

export const STATE_TEXT: Record<OnceState, string> = {
  funded: "买家已付款，等待卖家发货",
  delivered: "卖家已发货，等待买家确认收货",
  released: "已确认收货，货款已放给卖家",
  refunded: "已退款给买家",
  settled: "已协商结算",
  disputed: "争议中",
};

/**
 * 确认级别（plan 第 6 节）：latest / safe 界面即时显示但可能被回滚，标「处理中」；finalized 才是最终结果。
 */
export function levelBadge(o: OrderView): { label: string; tone: BadgeTone } {
  return o.level === "finalized"
    ? { label: "已最终确认", tone: "success" }
    : { label: "已上链（处理中）", tone: "warning" };
}

const TERMINAL = new Set(["released", "refunded", "settled"]);

/** 结局已定且已最终确认：不会再变，订单页停止轮询。 */
export function isFinal(o: OrderView): boolean {
  return TERMINAL.has(o.state) && o.level === "finalized";
}

export type OrderAction =
  { kind: "markDelivered"; label: "标记已发货" } | { kind: "release"; label: "确认收货" };

/**
 * 当前钱包在这单上能做的操作，与合约条件一致：卖家在发货期限内、尚未发货时标记发货；
 * 买家在已付款状态随时可以确认收货（`release`）。
 */
export function availableAction(o: OrderView, me: string, now: number): OrderAction | null {
  if (o.state !== "funded") return null;
  const mine = (a: string) => a.toLowerCase() === me.toLowerCase();
  const { ship, confirm } = o.deadlines;
  const shipOpen = !ship || Date.parse(ship) >= now;
  if (mine(o.seller.address) && !confirm && shipOpen) {
    return { kind: "markDelivered", label: "标记已发货" };
  }
  if (mine(o.buyer.address)) return { kind: "release", label: "确认收货" };
  return null;
}

/** 截止时间 → 「还剩 2 天 3 小时」/「已到期」，冷却与期限要显示倒计时（plan 第 14 节）。 */
export function remaining(deadline: string, now: number): string {
  const secs = Math.floor((Date.parse(deadline) - now) / 1000);
  if (!Number.isFinite(secs)) return "";
  if (secs <= 0) return "已到期";
  const d = Math.floor(secs / 86_400);
  const h = Math.floor((secs % 86_400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (d > 0) return `还剩 ${d} 天 ${h} 小时`;
  if (h > 0) return `还剩 ${h} 小时 ${m} 分`;
  return `还剩 ${Math.max(m, 1)} 分钟`;
}
