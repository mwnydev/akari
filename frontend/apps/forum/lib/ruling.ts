// 裁决比例与金额（plan 第 7 节）：裁决 = 链上 `resolve(orderId, toBuyerBps, bondToBuyerBps)` 的两个比例。
// 这里的预估只给滑块即时显示；签署前以服务端按链上订单算出的金额（`ruling/options` 的 intent）为准。

import type { Schemas } from "@akari/api-client";

export const BPS = 10_000;

export interface Allocation {
  principalToBuyer: bigint;
  principalToSeller: bigint;
  bondToBuyer: bigint;
}

/** 与合约、后端 `forum_core::arbitration::allocate` 相同的取整：卖家份额向下取整，余下归买家。 */
export function allocate(
  principal: bigint,
  bond: bigint,
  toBuyerBps: number,
  bondToBuyerBps: number,
): Allocation {
  const seller = (principal * BigInt(BPS - toBuyerBps)) / BigInt(BPS);
  return {
    principalToBuyer: principal - seller,
    principalToSeller: seller,
    bondToBuyer: (bond * BigInt(bondToBuyerBps)) / BigInt(BPS),
  };
}

/**
 * 争议中可分配的本金：一次性订单为订单金额；分期订单是「可解锁净本金 − 已领取」，订单视图里没有净本金，
 * 前端算不出，返回 null（只显示服务端金额）。
 */
export function disputedPrincipal(order: Schemas["OrderView"]): bigint | null {
  return order.mode === "escrow" ? BigInt(order.amount) : null;
}

/** 随订单锁定的卖家保证金（没有为 0）。 */
export function orderBond(order: Schemas["OrderView"]): bigint {
  return BigInt(order.fees.bond ?? "0");
}

/** 「62.5」→ 6250；不是 0–100 之间、最多两位小数的数返回 null。 */
export function percentToBps(raw: string): number | null {
  const t = raw.trim();
  if (!/^\d{1,3}(\.\d{1,2})?$/.test(t)) return null;
  const [int = "0", frac = ""] = t.split(".");
  const bps = Number(int) * 100 + Number(frac.padEnd(2, "0"));
  return bps <= BPS ? bps : null;
}

/** 6250 → 「62.5%」。 */
export function bpsLabel(bps: number): string {
  return `${(bps / 100).toFixed(2).replace(/\.?0+$/, "")}%`;
}

/** 服务端签署内容（`RulingIntent` 的规范 JSON）里给人看的部分。 */
export interface SignedIntent {
  caseId: number;
  kind: Schemas["RulingKind"];
  toBuyerBps: number;
  bondToBuyerBps: number;
  principalToBuyer: string;
  principalToSeller: string;
  bondToBuyer: string;
}

/** 解析 `ruling/options` 返回的 intent；字段对不上返回 null（不签）。 */
export function parseIntent(json: string): SignedIntent | null {
  let v: unknown;
  try {
    v = JSON.parse(json);
  } catch {
    return null;
  }
  if (typeof v !== "object" || v === null) return null;
  const o = v as Record<string, unknown>;
  const str = (k: string) => (typeof o[k] === "string" && /^\d+$/.test(o[k]) ? o[k] : null);
  const num = (k: string) => (typeof o[k] === "number" ? o[k] : null);
  const kind = o.kind;
  const [caseId, toBuyerBps, bondToBuyerBps] = [
    num("case_id"),
    num("to_buyer_bps"),
    num("bond_to_buyer_bps"),
  ];
  const [principalToBuyer, principalToSeller, bondToBuyer] = [
    str("principal_to_buyer"),
    str("principal_to_seller"),
    str("bond_to_buyer"),
  ];
  if (
    caseId === null ||
    toBuyerBps === null ||
    bondToBuyerBps === null ||
    principalToBuyer === null ||
    principalToSeller === null ||
    bondToBuyer === null ||
    (kind !== "panel" && kind !== "owner" && kind !== "appeal")
  ) {
    return null;
  }
  return {
    caseId,
    kind,
    toBuyerBps,
    bondToBuyerBps,
    principalToBuyer,
    principalToSeller,
    bondToBuyer,
  };
}

/** 签署内容必须正是表单里填的那份（服务端也会按提交内容重算 challenge）。 */
export function intentMatches(
  intent: SignedIntent,
  form: { caseId: number; kind: Schemas["RulingKind"]; toBuyerBps: number; bondToBuyerBps: number },
): boolean {
  return (
    intent.caseId === form.caseId &&
    intent.kind === form.kind &&
    intent.toBuyerBps === form.toBuyerBps &&
    intent.bondToBuyerBps === form.bondToBuyerBps
  );
}
