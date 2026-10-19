import { Badge, Countdown, cn } from "@akari/ui";
import { Clock } from "lucide-react";
import type { AuctionFacts } from "../_mock/data";
import { formatAuctionAmount } from "./money";

/** 剩余不到 1 小时的拍卖用警示色提示「即将结束」。 */
const ENDING_SOON = 3600;

/** 主题列表行里拍卖帖的信息：当前价、出价次数、剩余时间、是否达到保留价（拍卖还没有接口，只在设计稿里）。 */
export function AuctionLine({ auction }: { auction: AuctionFacts }) {
  const soon = auction.secondsLeft < ENDING_SOON;
  return (
    <>
      <span>
        当前{" "}
        <span className="amount font-medium text-fg">
          {formatAuctionAmount(auction.currency, BigInt(auction.current))}
        </span>
      </span>
      <span className="amount">{auction.bids} 次出价</span>
      <span className={cn("inline-flex items-center gap-1", soon && "font-medium text-warning")}>
        <Clock size={13} aria-hidden />
        {soon ? "即将结束 " : "剩 "}
        <Countdown seconds={auction.secondsLeft} />
      </span>
      <Badge tone={auction.reserveMet ? "success" : "neutral"}>
        {auction.reserveMet ? "已达保留价" : "未达保留价"}
      </Badge>
    </>
  );
}
