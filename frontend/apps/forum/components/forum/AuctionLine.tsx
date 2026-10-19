import { Badge, Countdown, cn } from "@akari/ui";
import { Clock } from "lucide-react";
import { formatAuctionAmount, type AuctionCurrency } from "../../lib/auction";

/** 剩余不到 1 小时的拍卖用警示色提示「即将结束」。 */
const ENDING_SOON = 3600;

/** 拍卖帖在列表里的摘要。 */
export interface AuctionFacts {
  currency: AuctionCurrency;
  /** 当前价（还没人出价时为起拍价）：USDT 时为最小单位，CNY 时为分 */
  current: string;
  bids: number;
  /** 渲染时距结束的秒数；0 = 已结束 */
  secondsLeft: number;
  reserveMet: boolean;
}

/** 主题列表行里拍卖帖的信息：当前价、出价次数、剩余时间、是否达到保留价。 */
export function AuctionLine({ auction }: { auction: AuctionFacts }) {
  const soon = auction.secondsLeft > 0 && auction.secondsLeft < ENDING_SOON;
  return (
    <>
      <span>
        当前{" "}
        <span className="amount font-medium text-fg">
          {formatAuctionAmount(auction.currency, BigInt(auction.current))}
        </span>
      </span>
      <span className="amount">{auction.bids} 次出价</span>
      {auction.secondsLeft > 0 ? (
        <span className={cn("inline-flex items-center gap-1", soon && "font-medium text-warning")}>
          <Clock size={13} aria-hidden />
          {soon ? "即将结束 " : "剩 "}
          <Countdown seconds={auction.secondsLeft} />
        </span>
      ) : (
        <span>已结束</span>
      )}
      <Badge tone={auction.reserveMet ? "success" : "neutral"}>
        {auction.reserveMet ? "已达保留价" : "未达保留价"}
      </Badge>
    </>
  );
}
