import type { Schemas } from "@akari/api-client";
import { DEFAULT_PARAMS } from "@akari/fees";
import { Button } from "@akari/ui";
import { ShieldCheck } from "lucide-react";
import type { Metadata } from "next";
import { BottomComposer } from "../../../../components/forum/BottomComposer";
import { ComposerProvider } from "../../../../components/forum/ComposerContext";
import { FloorStream } from "../../../../components/forum/Floor";
import { SellerCard } from "../../../../components/forum/SellerCard";
import { TopicHeader } from "../../../../components/forum/TopicHeader";
import { TradeActions } from "../../../../components/forum/TradeActions";
import { TradePanel, type ListingTermsView } from "../../../../components/forum/TradePanel";
import { OfferCard } from "../../../../components/trade/OfferCard";
import { orderCostPreview } from "../../../../lib/fees";
import { MockFloor } from "../_components/MockFloor";
import { MockShell } from "../_components/MockShell";
import { topics, tradeFloors, user, VIEWER_ID } from "../_mock/data";
import { mockHeader, mockPerson } from "../_mock/views";

export const metadata: Metadata = { title: "交易帖" };

const DAY = 86_400;

const PRICE = "45000000";

const terms: ListingTermsView = {
  price: PRICE,
  cnyPerUsdt: 72_000,
  instant: true,
  shipWindow: 0,
  confirmWindow: 3 * DAY,
  bond: "10000000",
  stock: 37,
  feeLabel: "卖家承担",
  buyerPays: orderCostPreview(PRICE, "seller", {
    fee_bps: Number(DEFAULT_PARAMS.feeBps),
    min_fee: DEFAULT_PARAMS.minFee.toString(),
    base_gas: DEFAULT_PARAMS.baseGas.toString(),
  }).buyerPays,
};

/** 4 楼嵌入的批量公开报价（卖家签名内容的模拟值；页面只读其中显示用的字段）。 */
const bulkOffer: Schemas["OfferView"] = {
  id: 9001,
  chain_id: 42161,
  created_at: "2026-10-20T02:12:00Z",
  expires_at: "2026-10-27T02:12:00Z",
  listing_id: 102,
  offer_hash: "0x",
  seller_id: 8,
  signature: "0x",
  status: "pending",
  verifying_contract: "0x0000000000000000000000000000000000000000",
  offer: {
    amount: "430000000",
    bond: "0",
    buyer: "0x0000000000000000000000000000000000000000",
    confirmWindow: 3 * DAY,
    expiry: 1_792_980_720,
    feePayer: 0,
    instant: true,
    lateSlashBps: 0,
    listingHash: "0x",
    maxBaseGas: "100000",
    maxFeeBps: 100,
    maxFills: 5,
    maxMinFee: "1000000",
    nonce: "1",
    seller: "0x0000000000000000000000000000000000000000",
    shipWindow: 0,
    token: "0x0000000000000000000000000000000000000000",
  },
};

/** 正文列与右侧面板；窄屏面板排在标题和楼层之间。 */
const columns = "xl:grid xl:grid-cols-[minmax(0,1fr)_21rem] xl:grid-rows-[auto_1fr] xl:gap-x-8";

export default function DesignTradePage() {
  const topic = topics.find((t) => t.id === 102);
  if (!topic) throw new Error("mock topic 102 missing");
  return (
    <ComposerProvider>
      <MockShell page="/design/v2/trade" active={topic.board} className={columns}>
        <div className="min-w-0 xl:col-start-1 xl:row-start-1">
          <TopicHeader topic={mockHeader(topic)} />
        </div>
        <aside
          aria-label="价格与条款"
          className="mt-3 space-y-3 xl:col-start-2 xl:row-span-2 xl:row-start-1 xl:mt-0"
        >
          <TradePanel
            terms={terms}
            actions={
              <TradeActions
                listingId={topic.id}
                sellerId={topic.authorId}
                viewerId={VIEWER_ID}
                offerId={bulkOffer.id}
                here="/design/v2/trade"
              />
            }
          />
          <SellerCard
            seller={mockPerson(topic.authorId)}
            facts={`在售 12 件 · 加入于 ${user(topic.authorId).joined}`}
          />
        </aside>
        <div className="mt-4 min-w-0 xl:col-start-1 xl:row-start-2 xl:mt-0">
          <FloorStream label="说明与讨论">
            {tradeFloors.map((f) => (
              <MockFloor
                key={f.floor}
                page="/design/v2/trade"
                data={f}
                opAuthorId={topic.authorId}
                extra={
                  f.hasOffer ? (
                    <div className="max-w-sm">
                      <OfferCard offer={bulkOffer} canPay={false}>
                        <p className="text-xs text-fg-muted">
                          公开报价 · 限 5 份 · 条款以卖家签名为准
                        </p>
                        <Button size="lg" className="w-full">
                          担保购买 10 张
                        </Button>
                      </OfferCard>
                    </div>
                  ) : null
                }
              />
            ))}
          </FloorStream>
          <p className="mt-3 text-xs text-fg-muted">
            内容快照 0x7c1e9a40d2…3b8f61aa ·
            报价签名锁定标题、说明、价格与条款，卖家之后编辑帖子不影响已签报价与已成交订单
          </p>
        </div>
      </MockShell>
      <BottomComposer
        mode={{ kind: "reply", topicId: topic.id }}
        viewer={{ name: user(VIEWER_ID).name }}
        action={
          <Button size="sm" className="xl:hidden">
            <ShieldCheck size={16} aria-hidden />
            担保购买
          </Button>
        }
      />
    </ComposerProvider>
  );
}
