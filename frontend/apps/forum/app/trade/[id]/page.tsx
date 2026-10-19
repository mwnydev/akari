import { ApiError } from "@akari/api-client";
import { Amount, Badge, Card, EmptyState, ErrorState, NetworkBadge, UserCard } from "@akari/ui";
import { Clock, ShieldCheck, Zap } from "lucide-react";
import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";
import { cache } from "react";
import { PresenceDot } from "../../../components/live/PresenceDot";
import { PageError } from "../../../components/PageError";
import { Prose } from "../../../components/Prose";
import { SiteNav } from "../../../components/SiteNav";
import { OfferCard } from "../../../components/trade/OfferCard";
import { load, serverApi } from "../../../lib/api";
import {
  cardLookup,
  formatDays,
  profileHref,
  tradeReputation,
  userName,
} from "../../../lib/display";
import {
  cnyReference,
  FEE_PAYERS,
  feePayerOf,
  formatCnyPerUsdt,
  orderCostPreview,
} from "../../../lib/fees";
import { boardHref, topicHref } from "../../../lib/links";

type Params = Promise<{ id: string }>;

/** 交易帖 = 主题（标题、正文）+ 交易条款；两者都按主题的可见性返回。 */
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

export async function generateMetadata({ params }: { params: Params }): Promise<Metadata> {
  const data = await loadListing(listingId((await params).id));
  return data instanceof ApiError ? {} : { title: data.topic.topic.title };
}

export default async function TradePage({ params }: { params: Params }) {
  const data = await loadListing(listingId((await params).id));
  if (data instanceof ApiError) {
    if (data.status === 404) notFound();
    return (
      <>
        <SiteNav />
        <PageError error={data} />
      </>
    );
  }
  const { listing, topic, offers, params: tradeParams } = data;
  const { forum } = topic;
  const first = topic.posts.find((p) => p.floor === 1);
  const seller = cardLookup(topic.users)(listing.seller_id);
  const terms = listing.terms;
  const feePayer = feePayerOf(terms.fee_payer);
  const cost =
    tradeParams instanceof ApiError ? null : orderCostPreview(listing.price, feePayer, tradeParams);
  const bond = terms.bond ?? "0";

  return (
    <>
      <SiteNav active={boardHref(forum.slug)} />
      <main className="mx-auto grid max-w-5xl gap-4 px-4 py-5 lg:grid-cols-[1fr_20rem]">
        <section className="min-w-0 space-y-3">
          <nav aria-label="位置" className="text-sm text-fg-muted">
            <Link href="/" className="hover:text-fg">
              首页
            </Link>{" "}
            /{" "}
            <Link href={boardHref(forum.slug)} className="hover:text-fg">
              {forum.name}
            </Link>
          </nav>
          <h1 className="text-2xl leading-snug font-semibold">
            <Badge tone="warning" className="mr-2 align-[4px]">
              担保
            </Badge>
            {topic.topic.title}
          </h1>
          <Card>
            <UserCard
              name={userName(seller)}
              avatar={seller.avatar_url ?? undefined}
              level={seller.level}
              reputation={tradeReputation(seller.reputation)}
              href={profileHref(seller)}
              avatarBadge={<PresenceDot userId={seller.id} avatarSize={44} />}
            />
            <p className="mt-1 text-xs text-fg-muted">
              交易信誉只统计链上已最终确认的订单；详细记录见卖家主页
            </p>
            <div className="mt-4">
              {first && !first.deleted ? (
                <Prose html={first.html} />
              ) : (
                <p className="text-fg-muted italic">卖家已删除说明</p>
              )}
            </div>
            <p className="mt-4 text-xs break-all text-fg-muted">
              内容快照 <span className="amount">{shortHash(listing.content_hash)}</span> ·
              报价签名锁定这份标题、说明、价格与条款，卖家之后编辑帖子不影响已签报价与已成交订单
            </p>
          </Card>
          <p className="text-sm">
            <Link href={topicHref(topic.topic.id)} className="text-accent hover:underline">
              查看讨论（{topic.topic.replies} 条回复）
            </Link>
          </p>
        </section>
        <aside className="min-w-0 space-y-3">
          <Card className="space-y-4">
            <div>
              <p className="text-sm text-fg-muted">标价</p>
              <p className="text-3xl font-semibold">
                <Amount value={listing.price} />
              </p>
              {listing.cny_per_usdt ? (
                <p className="mt-1 text-sm text-fg-muted">
                  ≈ {cnyReference(listing.price, listing.cny_per_usdt)}（卖家参考汇率{" "}
                  {formatCnyPerUsdt(listing.cny_per_usdt)}）
                </p>
              ) : null}
            </div>
            <ul className="space-y-2 text-sm">
              <li className="flex items-center gap-2">
                <Zap size={16} className="text-accent" aria-hidden />
                {terms.instant
                  ? "即时发货：付款确认后自动发货"
                  : `卖家 ${formatDays(terms.ship_window)}内发货`}
              </li>
              <li className="flex items-center gap-2">
                <Clock size={16} className="text-accent" aria-hidden />
                买家确认期 {formatDays(terms.confirm_window)}，期满自动放款
              </li>
              <li className="flex items-center gap-2">
                <ShieldCheck size={16} className="text-accent" aria-hidden />
                {bond === "0" ? (
                  "无卖家保证金"
                ) : (
                  <span>
                    卖家保证金 <Amount value={bond} />
                    ，随订单锁定
                  </span>
                )}
              </li>
            </ul>
            {cost ? (
              <dl className="space-y-1 rounded-lg bg-surface-2 p-3 text-sm">
                <div className="flex justify-between gap-2">
                  <dt className="text-fg-muted">手续费</dt>
                  <dd>{FEE_PAYERS[terms.fee_payer ?? 0]?.label}</dd>
                </div>
                <div className="flex justify-between gap-2">
                  <dt className="text-fg-muted">卖家承担的手续费与 gas</dt>
                  <dd>
                    <Amount value={cost.sellerFee} />
                  </dd>
                </div>
                <div className="flex justify-between gap-2">
                  <dt className="text-fg-muted">你需支付（含 0.05 代付费）</dt>
                  <dd className="font-medium">
                    <Amount value={cost.buyerPays} />
                  </dd>
                </div>
                <div className="flex justify-between gap-2">
                  <dt className="text-fg-muted">本单争议费（发起方预付）</dt>
                  <dd>
                    <Amount value={cost.disputeFee} />
                  </dd>
                </div>
              </dl>
            ) : null}
            <NetworkBadge />
          </Card>
          <h2 className="font-semibold">卖家报价</h2>
          {offers instanceof ApiError ? (
            <Card>
              <ErrorState title="报价加载失败" message={offers.message} />
            </Card>
          ) : offers.length ? (
            offers.map((o) => <OfferCard key={o.id} offer={o} />)
          ) : (
            <Card>
              <EmptyState
                title="卖家还没有发布报价"
                description="成交以卖家在钱包里签名的报价为准；报价发布后这里会出现「担保购买」。"
              />
            </Card>
          )}
        </aside>
      </main>
    </>
  );
}
