import { Amount } from "@akari/ui";
import { formatCnyFen } from "../lib/cny";
import type { AuctionAuth } from "../lib/wallet/auction";

// 拍卖签名页与付款页共用的显示。只引用类型：解析与核对都在钱包 SDK 里，页面不另带一份。

/** 链接里的拍卖编号（站内拍卖 ID，即授权里的 auctionId；页面按 JS 数字处理，限 15 位以内）。 */
export const AUCTION_ID = /^[1-9]\d{0,14}$/;

/** 拍卖币种金额 → 「¥1,280.00」或「128.00 USDT」。 */
export function AuctionPrice({ cny, amount }: { cny: boolean; amount: bigint }) {
  return cny ? <span className="amount">{formatCnyFen(amount)}</span> : <Amount value={amount} />;
}

/** 整天显示「3 天」，否则「36 小时」。 */
export function durationText(secs: number): string {
  return secs % 86_400 === 0 ? `${secs / 86_400} 天` : `${Math.round(secs / 3600)} 小时`;
}

/** 发货期 / 确认期：「即时发货 / 7 天」「3 天 / 7 天」。 */
export function windowsText(auth: Pick<AuctionAuth, "instant" | "shipWindow" | "confirmWindow">) {
  return `${auth.instant ? "即时发货" : durationText(auth.shipWindow)} / ${durationText(auth.confirmWindow)}`;
}
