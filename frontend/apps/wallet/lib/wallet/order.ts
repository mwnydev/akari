import type { BadgeTone, OnceState } from "@akari/ui";
import type { OrderView } from "./api";
import { StreamState, canStart, claimCheck, type ClaimCheck, type StreamOrder } from "./stream";

// 订单页的展示与按钮规则（纯函数，不依赖钱包 SDK，可以进首屏 JS）。数据来自 indexer（GET /api/v1/orders/{id}）。
// 订单 ID 是站内 ID；链上操作用 `chain_order_id` + `contract`（两份担保合约各自编号，链上订单号会重复）。

/** 一次性担保订单（`mode = escrow`）；否则是分期担保（`stream`）。 */
export function isOnce(o: OrderView): boolean {
  return o.mode === "escrow";
}

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

const STATE_TEXT: Record<OnceState, string> = {
  funded: "买家已付款，等待卖家发货",
  delivered: "卖家已发货，等待买家确认收货",
  released: "已确认收货，货款已放给卖家",
  refunded: "已退款给买家",
  settled: "已协商结算",
  disputed: "争议中",
};

const STREAM_TEXT: Record<string, string> = {
  disputed: "争议中",
  released: "已全部解锁给卖家",
  refunded: "已退款给买家",
  settled: "已协商结算",
};

/** 列表与订单页的一句话状态；回滚撤销的订单返回 null。 */
export function stateText(o: OrderView): string | null {
  if (isOnce(o)) {
    const s = onceState(o);
    return s && STATE_TEXT[s];
  }
  if (o.state === "funded") {
    return o.stream?.started_at ? "服务进行中，按期解锁给卖家" : "买家已付款，等待卖家开始服务";
  }
  return STREAM_TEXT[o.state] ?? null;
}

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
  | { kind: "markDelivered"; label: "标记已发货" }
  | { kind: "release"; label: "确认收货" }
  | { kind: "start"; label: "开始服务" }
  /** 领取按钮一直显示，条件不满足时禁用并说明原因 */
  | { kind: "claim"; label: "领取已解锁部分" | "领取剩余并结清"; claim: ClaimCheck };

const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

/**
 * 当前钱包在这单上能做的操作，与合约条件一致：卖家在发货期限内、尚未发货时标记发货；
 * 买家在已付款状态随时可以确认收货（`release`）。只对链配置里那份 Escrow 上的一次性订单给按钮。
 */
export function availableAction(
  o: OrderView,
  me: string,
  now: number,
  escrow: string,
): OrderAction | null {
  const mine = (a: string) => same(a, me);
  if (!isOnce(o) || !same(o.contract, escrow)) return null;
  if (o.state !== "funded") return null;
  const { ship, confirm } = o.deadlines;
  const shipOpen = !ship || Date.parse(ship) >= now;
  if (mine(o.seller.address) && !confirm && shipOpen) {
    return { kind: "markDelivered", label: "标记已发货" };
  }
  if (mine(o.buyer.address)) return { kind: "release", label: "确认收货" };
  return null;
}

/**
 * 分期订单卖家能做的操作（买家只看进度）：开始期限内 `start`；服务进行中 `claim`（条件见 `claimCheck`）。
 * `chainOrder` 是链上 latest 的订单（还没读到时为 undefined，不给按钮）。只对链配置里那份 StreamEscrow 给按钮。
 */
export function streamAction(
  o: OrderView,
  chainOrder: StreamOrder | undefined,
  me: string,
  now: number,
  streamEscrow: string,
): OrderAction | null {
  if (isOnce(o) || !same(o.contract, streamEscrow) || !chainOrder) return null;
  if (!same(o.seller.address, me) || chainOrder.state !== StreamState.Funded) return null;
  const t = BigInt(Math.floor(now / 1000));
  if (canStart(chainOrder, t)) return { kind: "start", label: "开始服务" };
  if (chainOrder.startedAt === 0n) return null;
  const claim = claimCheck(chainOrder, t);
  return {
    kind: "claim",
    label: claim.ok && claim.settles ? "领取剩余并结清" : "领取已解锁部分",
    claim,
  };
}

/**
 * 订单「版本」：提交上链后记下当时的版本，indexer 反映出变化（状态、确认期限、开始时间、已领取、更新区块）
 * 之前不再给按钮，避免重复提交。
 */
export function orderVersion(o: OrderView): string {
  return [
    o.state,
    o.updated_block,
    o.deadlines.confirm,
    o.stream?.started_at,
    o.stream?.claimed,
  ].join("|");
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
