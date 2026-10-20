import { ApiError } from "@akari/api-client";
import { Button, Card, ErrorState } from "@akari/ui";
import { ShieldCheck } from "lucide-react";
import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";
import { cache } from "react";
import { AppShell } from "../../../components/forum/AppShell";
import { BottomComposer } from "../../../components/forum/BottomComposer";
import { ComposerProvider } from "../../../components/forum/ComposerContext";
import { FloorStream } from "../../../components/forum/Floor";
import { LiveFloors } from "../../../components/forum/LiveFloor";
import { ReplyNotice } from "../../../components/forum/ReplyNotice";
import { SellerCard } from "../../../components/forum/SellerCard";
import { TopicHeader } from "../../../components/forum/TopicHeader";
import { TradeActions } from "../../../components/forum/TradeActions";
import { TradePanel } from "../../../components/forum/TradePanel";
import { PresenceDot } from "../../../components/live/PresenceDot";
import { PageError } from "../../../components/PageError";
import { OfferCard } from "../../../components/trade/OfferCard";
import { load, serverApi } from "../../../lib/api";
import { cardLookup, formatTime } from "../../../lib/display";
import { FEE_PAYERS, feePayerOf, orderCostPreview } from "../../../lib/fees";
import { topicHref } from "../../../lib/links";
import { headerViewer } from "../../../lib/nav";
import { WALLET_ORIGIN, needsPostCaptcha } from "../../../lib/public-config";
import { getMe } from "../../../lib/session";
import { personCard, topicHeaderView } from "../../../lib/topic-views";
import { WalletLink } from "../../../components/settings/UsernameGate";

type Params = Promise<{ id: string }>;

/** 交易帖 = 主题（标题、正文与第 1 页讨论）+ 交易条款；两者都按主题的可见性返回。 */
const loadListing = cache(async (id: number) => {
  const api = await serverApi();
  const listing = await load(api.GET("/api/v1/listings/{id}", { params: { path: { id } } }));
  if (listing instanceof ApiError) return listing;
  const [topic, offers, params] = await Promise.all([
    load(api.GET("/api/v1/topics/{id}", { params: { path: { id: listing.topic_id } } })),
    load(api.GET("/api/v1/listings/{id}/offers", { params: { path: { id } } })),
    load(api.GET("/api/v1/trade/params")),
  ]);
  if (topic instanceof ApiError) return topic;
  return { listing, topic, offers, params };
});

function listingId(raw: string): number {
  if (!/^\d{1,15}$/.test(raw)) notFound();
  return Number(raw);
}

/** 内容哈希显示前 10 位与后 8 位。 */
function shortHash(h: string): string {
  return h.length > 20 ? `${h.slice(0, 10)}…${h.slice(-8)}` : h;
}

/** 正文列与右侧面板；窄屏面板排在标题和楼层之间。 */
const columns = "xl:grid xl:grid-cols-[minmax(0,1fr)_21rem] xl:grid-rows-[auto_1fr] xl:gap-x-8";

export async function generateMetadata({ params }: { params: Params }): Promise<Metadata> {
  const data = await loadListing(listingId((await params).id));
  return data instanceof ApiError ? {} : { title: data.topic.topic.title };
}

export default async function TradePage({ params }: { params: Params }) {
  const id = listingId((await params).id);
  const [data, me] = await Promise.all([loadListing(id), getMe()]);
  if (data instanceof ApiError) {
    if (data.status === 404) notFound();
    return (
      <AppShell>
        <PageError error={data} />
      </AppShell>
    );
  }
  const { listing, topic: page, offers, params: tradeParams } = data;
  const { forum, topic } = page;
  const user = cardLookup(page.users);
  const seller = user(listing.seller_id);
  const terms = listing.terms;
  const cost =
    tradeParams instanceof ApiError
      ? null
      : orderCostPreview(listing.price, feePayerOf(terms.fee_payer), tradeParams);
  const offer = offers instanceof ApiError ? undefined : offers.find((o) => o.status === "pending");
  const mentionable = page.users.filter((u) => u.id !== me?.id);
  const isSeller = me?.id === listing.seller_id;
  const here = `/trade/${id}`;
  const now = new Date();

  return (
    <ComposerProvider captcha={needsPostCaptcha(me)}>
      <AppShell active={forum.slug} className={columns}>
        <div className="min-w-0 xl:col-start-1 xl:row-start-1">
          <TopicHeader topic={topicHeaderView(topic, forum)} />
        </div>
        <aside
          aria-label="价格与条款"
          className="mt-3 space-y-3 xl:col-start-2 xl:row-span-2 xl:row-start-1 xl:mt-0"
        >
          <TradePanel
            terms={{
              price: listing.price,
              cnyPerUsdt: listing.cny_per_usdt,
              instant: terms.instant ?? false,
              shipWindow: terms.ship_window,
              confirmWindow: terms.confirm_window,
              bond: terms.bond ?? "0",
              feeLabel: FEE_PAYERS[terms.fee_payer ?? 0]?.label ?? "卖家承担",
              buyerPays: cost?.buyerPays,
            }}
            actions={
              <TradeActions
                listingId={id}
                sellerId={listing.seller_id}
                viewerId={me?.id ?? null}
                offerId={offer?.id}
                here={here}
              />
            }
          />
          {offers instanceof ApiError ? (
            <Card>
              <ErrorState title="报价加载失败" message={offers.message} />
            </Card>
          ) : offers.length ? (
            <section aria-label="卖家报价" className="space-y-3">
              <h2 className="text-sm font-medium text-fg-muted">卖家报价</h2>
              {offers.map((o) => (
                <OfferCard key={o.id} offer={o} canPay={!isSeller} />
              ))}
            </section>
          ) : null}
          <SellerCard
            seller={personCard(seller)}
            avatarBadge={<PresenceDot userId={seller.id} avatarSize={44} />}
            facts={`本帖发布于 ${formatTime(listing.created_at).slice(0, 10)}`}
          />
        </aside>
        <div className="mt-4 min-w-0 xl:col-start-1 xl:row-start-2 xl:mt-0">
          <FloorStream label="说明与讨论">
            <LiveFloors data={page} me={me} page={1} now={now} />
          </FloorStream>
          {page.total_pages > 1 ? (
            <p className="py-3 text-center text-sm">
              <Link href={topicHref(topic.id, 2)} className="text-accent hover:underline">
                查看更多回复（共 {topic.replies} 条）
              </Link>
            </p>
          ) : null}
          <p className="mt-3 text-xs break-all text-fg-muted">
            内容快照 <span className="amount">{shortHash(listing.content_hash)}</span> ·
            报价签名锁定标题、说明、价格与条款，卖家之后编辑帖子不影响已签报价与已成交订单
          </p>
        </div>
      </AppShell>
      <BottomComposer
        mode={{ kind: "reply", topicId: topic.id }}
        viewer={me ? headerViewer(me) : null}
        mentionable={mentionable}
        blocked={
          page.can_reply ? undefined : (
            <ReplyNotice signedIn={me !== null} locked={topic.flags.locked} here={here} />
          )
        }
        action={
          offer && !isSeller ? (
            <Button asChild size="sm" className="xl:hidden">
              <WalletLink href={`${WALLET_ORIGIN}/pay?offer=${offer.id}`}>
                <ShieldCheck size={16} aria-hidden />
                担保购买
              </WalletLink>
            </Button>
          ) : null
        }
      />
    </ComposerProvider>
  );
}
