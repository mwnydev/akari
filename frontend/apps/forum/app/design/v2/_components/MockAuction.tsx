"use client";

import { SegmentedControl } from "@akari/ui/tabs";
import { useState } from "react";
import {
  AuctionPanel,
  type AuctionMyStatus,
  type AuctionPanelBid,
  type AuctionPanelData,
} from "../../../../components/forum/AuctionPanel";
import { minNextBid, type AuctionState } from "../../../../lib/auction";

type Phase = "live" | "sold" | "runner" | "unsold";

const PHASES = [
  { value: "live", label: "进行中" },
  { value: "sold", label: "成交待付款" },
  { value: "runner", label: "递补" },
  { value: "unsold", label: "流拍" },
] as const satisfies readonly { value: Phase; label: string }[];

const PHASE_VIEW: Record<Phase, { state: AuctionState; my: AuctionMyStatus }> = {
  live: { state: "live", my: "none" },
  sold: { state: "awaiting_payment", my: "winner" },
  runner: { state: "awaiting_payment", my: "runner_up_offered" },
  unsold: { state: "unsold", my: "none" },
};

/** 设计稿：正式拍卖面板 + 模拟数据，顶部切换预览各状态；出价只在本页记一笔，不发请求。 */
export function MockAuction({
  auction,
  bids: initialBids,
  viewer,
}: {
  auction: AuctionPanelData;
  bids: AuctionPanelBid[];
  /** 已脱敏的访问者名字，用于自己的出价记录 */
  viewer: string;
}) {
  const [phase, setPhase] = useState<Phase>("live");
  const [bids, setBids] = useState(initialBids);
  const top = bids[0];
  const current = top?.amount ?? auction.current;
  const data: AuctionPanelData = {
    ...auction,
    ...PHASE_VIEW[phase],
    ...(phase === "live" && top?.mine ? { my: "leading" as const } : {}),
    current,
    minNext: minNextBid({
      currency: auction.currency,
      cnyPerUsdt: auction.cnyPerUsdt,
      start: BigInt(auction.start),
      increment: BigInt(auction.increment),
      current: current === null ? null : BigInt(current),
    }).toString(),
    bidCount: auction.bidCount + bids.length - initialBids.length,
  };
  return (
    <AuctionPanel
      auction={data}
      bids={bids}
      payHref="#"
      access={{
        kind: "open",
        place: async (amount) => {
          setBids((bs) => [
            {
              id: (bs[0]?.id ?? 0) + 1,
              bidder: viewer,
              amount: amount.toString(),
              time: "刚刚",
              timeIso: new Date().toISOString(),
              mine: true,
              linked: false,
            },
            ...bs,
          ]);
        },
      }}
      header={
        <SegmentedControl
          label="预览拍卖状态（设计稿）"
          value={phase}
          onValueChange={setPhase}
          options={[...PHASES]}
          className="text-xs"
        />
      }
    />
  );
}
