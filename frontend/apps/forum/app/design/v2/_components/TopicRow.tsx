import { Amount, Avatar, AvatarStack, Badge, Countdown, cn, successRateLabel } from "@akari/ui";
import { Clock, Lock, Pin, Zap } from "lucide-react";
import { cnyReference } from "../../../../lib/fees";
import { board, user, type AuctionFacts, type TopicSummary, type TradeFacts } from "../_mock/data";
import { BoardMark } from "./BoardMark";
import { formatCount } from "./format";
import { KindBadge } from "./KindBadge";
import { formatAuctionAmount } from "./money";
import { UserPopover } from "./UserPopover";

/** 剩余不到 1 小时的拍卖用警示色提示「即将结束」。 */
const ENDING_SOON = 3600;

/** 回复数达到这些档位时换成「热度」色（从高到低匹配）。 */
const HEAT: [number, string][] = [
  [150, "text-heat-3"],
  [100, "text-heat-2"],
  [50, "text-heat-1"],
];

function heatClass(replies: number): string {
  const hit = HEAT.find(([min]) => replies >= min);
  return hit ? `font-medium ${hit[1]}` : "text-fg-muted";
}

function TradeLine({ trade }: { trade: TradeFacts }) {
  return (
    <>
      <span className="amount">
        <Amount value={trade.price} className="font-medium text-fg" />
        {trade.cnyPerUsdt ? ` ≈ ${cnyReference(trade.price, trade.cnyPerUsdt)}` : null}
      </span>
      <span className="amount">{successRateLabel(trade.seller)}</span>
      {trade.instant ? (
        <Badge tone="success">
          <Zap size={12} aria-hidden />
          自动发卡
        </Badge>
      ) : (
        <Badge>人工发货</Badge>
      )}
      {trade.stock !== undefined ? (
        <span className="amount">{trade.stock > 1 ? `库存 ${trade.stock}` : "仅 1 件"}</span>
      ) : null}
    </>
  );
}

function AuctionLine({ auction }: { auction: AuctionFacts }) {
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

const numCell = "amount hidden px-2 text-right md:table-cell";

/**
 * 主题列表一行（Discourse 式表格行，自有实现）。
 * 桌面端：主题、参与者、回复、浏览、活动五列；移动端：左侧作者头像，右侧回复数与最后活动。
 */
export function TopicRow({ topic: t }: { topic: TopicSummary }) {
  const author = user(t.authorId);
  return (
    <tr className="border-b border-border">
      <td className="py-3 pr-2 md:pl-2">
        <div className="flex gap-3">
          <span className="md:hidden">
            <UserPopover user={author}>
              <Avatar name={author.name} size={36} />
            </UserPopover>
          </span>
          <div className="min-w-0">
            <a
              href={t.href}
              className="rounded-sm text-[17px] leading-snug text-fg visited:text-fg-muted focus-visible:outline-2 focus-visible:outline-accent"
            >
              {t.pinned ? (
                <>
                  <Pin size={15} aria-hidden className="mr-1 inline align-[-2px] text-fg-muted" />
                  <span className="sr-only">置顶：</span>
                </>
              ) : null}
              {t.locked ? (
                <>
                  <Lock size={15} aria-hidden className="mr-1 inline align-[-2px] text-fg-muted" />
                  <span className="sr-only">已锁定：</span>
                </>
              ) : null}
              {t.title}
            </a>
            {t.unread ? (
              <span
                className="amount ml-1.5 inline-flex h-5 min-w-5 items-center justify-center rounded-full bg-accent-fill px-1.5 align-[2px] text-xs font-medium text-accent-fg"
                aria-label={`${t.unread} 条新回复`}
              >
                {t.unread}
              </span>
            ) : null}
            <p className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-sm text-fg-muted">
              <BoardMark board={board(t.board)} />
              {t.trade ? <KindBadge kind="trade" /> : null}
              {t.auction ? <KindBadge kind="auction" /> : null}
              {t.featured ? <Badge tone="success">精华</Badge> : null}
              {t.trade ? <TradeLine trade={t.trade} /> : null}
              {t.auction ? <AuctionLine auction={t.auction} /> : null}
            </p>
          </div>
        </div>
      </td>
      <td className="hidden px-2 md:table-cell">
        <AvatarStack people={t.participants.map((id) => ({ name: user(id).name }))} />
      </td>
      <td className={cn(numCell, heatClass(t.replies))}>{formatCount(t.replies)}</td>
      <td className={cn(numCell, "text-fg-muted")}>{formatCount(t.views)}</td>
      <td className="hidden pr-2 pl-2 text-right text-sm text-fg-muted md:table-cell">
        <time dateTime={t.activityIso}>{t.activity}</time>
      </td>
      <td className="py-3 text-right align-top text-sm md:hidden">
        <span
          className={cn("amount block", heatClass(t.replies))}
          aria-label={`${t.replies} 条回复`}
        >
          {formatCount(t.replies)}
        </span>
        <time dateTime={t.activityIso} className="block text-xs whitespace-nowrap text-fg-muted">
          {t.activity}
        </time>
      </td>
    </tr>
  );
}
