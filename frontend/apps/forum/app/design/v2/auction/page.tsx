import { Button } from "@akari/ui";
import { Gavel } from "lucide-react";
import type { Metadata } from "next";
import { auctionFloors, bids, topics, user, VIEWER_ID } from "../_mock/data";
import { AuctionPanel, type AuctionTerms } from "../_components/AuctionPanel";
import { AuctionRules } from "../_components/AuctionRules";
import { BottomComposer } from "../_components/BottomComposer";
import { ComposerProvider } from "../_components/ComposerContext";
import { Floor, FloorStream } from "../_components/Floor";
import { ForumShell } from "../_components/ForumShell";
import { SellerCard } from "../_components/SellerCard";
import { TopicHeader } from "../_components/TopicHeader";

export const metadata: Metadata = { title: "拍卖帖" };

/** 参数取 plan 第 9 节默认值：付款期 48 小时、防狙击 15 分钟、累计顺延上限 24 小时。 */
const terms: AuctionTerms = {
  currency: "CNY",
  cnyPerUsdt: 72_000,
  start: "50000",
  increment: "2000",
  current: "128000",
  bidCount: 23,
  reserveMet: true,
  secondsLeft: 2 * 3600 + 14 * 60 + 5,
  payHours: 48,
  paySecondsLeft: 47 * 3600 + 12 * 60 + 9,
  snipeMinutes: 15,
  snipeCapHours: 24,
  balancePercent: 10,
};

/** 正文列与右侧面板；窄屏面板排在标题和楼层之间。 */
const columns = "xl:grid xl:grid-cols-[minmax(0,1fr)_23rem] xl:grid-rows-[auto_1fr] xl:gap-x-8";

/** 出价记录里的访问者名字按同样规则脱敏：首字 + **。 */
function mask(name: string): string {
  return `${Array.from(name)[0] ?? ""}**`;
}

export default function DesignAuctionPage() {
  const topic = topics.find((t) => t.id === 103);
  if (!topic) throw new Error("mock topic 103 missing");
  const seller = user(topic.authorId);
  return (
    <ComposerProvider>
      <ForumShell page="/design/v2/auction" active={topic.board} className={columns}>
        <div className="min-w-0 xl:col-start-1 xl:row-start-1">
          <TopicHeader topic={topic} />
        </div>
        <aside
          aria-label="出价"
          className="mt-3 space-y-3 xl:col-start-2 xl:row-span-2 xl:row-start-1 xl:mt-0"
        >
          <AuctionPanel terms={terms} bids={bids} viewer={mask(user(VIEWER_ID).name)} />
          <AuctionRules terms={terms} />
          <SellerCard seller={seller} onSale={3} />
        </aside>
        <div className="mt-4 min-w-0 xl:col-start-1 xl:row-start-2 xl:mt-0">
          <FloorStream label="拍品说明与讨论">
            {auctionFloors.map((f) => (
              <Floor key={f.floor} data={f} opAuthorId={topic.authorId} />
            ))}
          </FloorStream>
        </div>
      </ForumShell>
      <BottomComposer
        viewer={user(VIEWER_ID).name}
        action={
          <Button asChild size="sm" className="xl:hidden">
            <a href="#bid-amount">
              <Gavel size={16} aria-hidden />
              出价
            </a>
          </Button>
        }
      />
    </ComposerProvider>
  );
}
