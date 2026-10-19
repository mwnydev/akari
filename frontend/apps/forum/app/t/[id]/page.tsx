import { ApiError } from "@akari/api-client";
import { Button, Card, ErrorState, Input } from "@akari/ui";
import { Gavel } from "lucide-react";
import type { Metadata } from "next";
import Link from "next/link";
import { notFound, redirect } from "next/navigation";
import { cache } from "react";
import { LiveAuction } from "../../../components/auction/LiveAuction";
import { AppShell } from "../../../components/forum/AppShell";
import { BottomComposer } from "../../../components/forum/BottomComposer";
import { ComposerProvider } from "../../../components/forum/ComposerContext";
import { FloorStream } from "../../../components/forum/Floor";
import { LiveFloors } from "../../../components/forum/LiveFloor";
import { ReplyNotice } from "../../../components/forum/ReplyNotice";
import { SellerCard } from "../../../components/forum/SellerCard";
import { Timeline } from "../../../components/forum/Timeline";
import { TopicHeader } from "../../../components/forum/TopicHeader";
import { PageError } from "../../../components/PageError";
import { Pager } from "../../../components/Pager";
import { TopicModMenu } from "../../../components/moderation/TopicModMenu";
import { load, serverApi } from "../../../lib/api";
import { auctionRulesParams } from "../../../lib/auction-view";
import { cardLookup, formatActivity } from "../../../lib/display";
import { listForums } from "../../../lib/forums";
import { loginHref, topicHref } from "../../../lib/links";
import { headerViewer } from "../../../lib/nav";
import { floorPage, parsePositiveInt } from "../../../lib/pagination";
import { needsPostCaptcha } from "../../../lib/public-config";
import { getMe } from "../../../lib/session";
import { isAuctionTopic, personCard, topicHeaderView } from "../../../lib/topic-views";

type Params = Promise<{ id: string }>;
type Search = Promise<Record<string, string | string[] | undefined>>;

/** 右侧时间轴的采样点；取不到时退回本页楼层。 */
async function loadTimeline(id: number) {
  return load(
    (await serverApi()).GET("/api/v1/topics/{id}/timeline", { params: { path: { id } } }),
  );
}

const loadTopic = cache(async (id: number, page: number) =>
  load(
    (await serverApi()).GET("/api/v1/topics/{id}", {
      params: { path: { id }, query: { page } },
    }),
  ),
);

/**
 * 拍卖帖右侧面板的首帧数据（之后由浏览器实时更新）：拍卖、第一页出价记录与站长设置的付款期等规则。
 * 失败返回 ApiError，面板显示出错，楼层照常可读。
 */
async function loadAuction(topicId: number) {
  const api = await serverApi();
  const [auction, params] = await Promise.all([
    load(api.GET("/api/v1/topics/{id}/auction", { params: { path: { id: topicId } } })),
    load(api.GET("/api/v1/auctions/params")),
  ]);
  if (auction instanceof ApiError) return auction;
  if (params instanceof ApiError) return params;
  const page = await load(
    api.GET("/api/v1/auctions/{id}/bids", { params: { path: { id: auction.id } } }),
  );
  if (page instanceof ApiError) return page;
  return { auction, bids: page.bids, rules: auctionRulesParams(params.settings) };
}

/** 拍卖帖：正文列与右侧出价面板；窄屏面板排在标题和楼层之间。 */
const auctionColumns =
  "xl:grid xl:grid-cols-[minmax(0,1fr)_23rem] xl:grid-rows-[auto_1fr] xl:gap-x-8";

function topicId(raw: string): number {
  if (!/^\d{1,15}$/.test(raw)) notFound();
  return Number(raw);
}

export async function generateMetadata({
  params,
  searchParams,
}: {
  params: Params;
  searchParams: Search;
}): Promise<Metadata> {
  const id = topicId((await params).id);
  const data = await loadTopic(id, parsePositiveInt((await searchParams).page));
  return data instanceof ApiError ? {} : { title: data.topic.title };
}

export default async function TopicPage({
  params,
  searchParams,
}: {
  params: Params;
  searchParams: Search;
}) {
  const id = topicId((await params).id);
  const sp = await searchParams;
  // 按楼层跳页（plan 第 4 节）：普通 GET 表单提交 ?floor=N，服务端换算页码后跳到该楼
  if (sp.floor !== undefined) {
    const floor = parsePositiveInt(sp.floor);
    redirect(topicHref(id, floorPage(floor), floor));
  }
  const page = parsePositiveInt(sp.page);
  const [data, me, timeline] = await Promise.all([loadTopic(id, page), getMe(), loadTimeline(id)]);
  if (data instanceof ApiError) {
    if (data.status === 404) notFound();
    return (
      <AppShell>
        <PageError error={data} />
      </AppShell>
    );
  }
  if (page > data.total_pages) redirect(topicHref(id, data.total_pages));

  const { topic, forum, posts, users } = data;
  const mentionable = users.filter((u) => u.id !== me?.id);
  const forums = data.can_moderate ? await listForums() : [];
  const now = new Date();
  const points =
    timeline instanceof ApiError
      ? posts.map((p) => ({ floor: p.floor, time: formatActivity(p.created_at, now) }))
      : timeline.points.map((p) => ({ floor: p.floor, time: formatActivity(p.at, now) }));
  const last = page === data.total_pages;

  const auction = isAuctionTopic(topic) ? await loadAuction(id) : null;
  const here = topicHref(id, page);
  const header = (
    <TopicHeader
      topic={topicHeaderView(topic, forum)}
      actions={
        data.can_moderate ? (
          <TopicModMenu
            topicId={topic.id}
            forumId={forum.id}
            flags={topic.flags}
            forums={forums instanceof ApiError ? [] : forums}
            staff={me?.staff}
          />
        ) : null
      }
      extra={data.total_pages > 1 ? ` · 第 ${page} / ${data.total_pages} 页` : null}
    />
  );
  const floors = (
    <>
      <FloorStream label={auction ? "拍品说明与讨论" : "楼层"}>
        <LiveFloors data={data} me={me} page={page} now={now} />
      </FloorStream>
      <Pager current={page} total={data.total_pages} href={(p) => topicHref(id, p)} />
      {data.total_pages > 1 ? (
        <form
          action={`/t/${id}`}
          method="get"
          className="flex items-center justify-center gap-2 text-sm whitespace-nowrap"
          aria-label="按楼层跳转"
        >
          <label htmlFor="jump-floor">跳到</label>
          <span className="w-28">
            <Input
              id="jump-floor"
              name="floor"
              type="number"
              inputMode="numeric"
              min={1}
              max={topic.last_floor}
              required
              className="h-8"
              placeholder={`1–${topic.last_floor}`}
            />
          </span>
          <span>楼</span>
          <Button type="submit" size="sm" variant="secondary">
            跳转
          </Button>
        </form>
      ) : null}
      {last ? (
        <p className="py-6 text-center text-sm text-fg-muted">
          已经到底了 · 共 {topic.last_floor} 层
        </p>
      ) : null}
    </>
  );

  return (
    <ComposerProvider captcha={needsPostCaptcha(me)}>
      {auction ? (
        <AppShell active={forum.slug} className={auctionColumns}>
          <div className="min-w-0 xl:col-start-1 xl:row-start-1">{header}</div>
          <aside
            aria-label="出价"
            className="mt-3 space-y-3 xl:col-start-2 xl:row-span-2 xl:row-start-1 xl:mt-0"
          >
            {auction instanceof ApiError ? (
              <Card>
                <ErrorState title="拍卖信息加载失败" message={auction.message} />
              </Card>
            ) : (
              <LiveAuction
                topicId={id}
                initial={auction.auction}
                initialBids={auction.bids}
                rules={auction.rules}
                signedIn={me !== null}
                login={loginHref(here)}
              />
            )}
            <SellerCard seller={personCard(cardLookup(users)(topic.author_id))} />
          </aside>
          <div className="mt-4 min-w-0 xl:col-start-1 xl:row-start-2 xl:mt-0">{floors}</div>
        </AppShell>
      ) : (
        <AppShell
          active={forum.slug}
          className="xl:grid xl:grid-cols-[minmax(0,48rem)_9rem] xl:gap-10"
        >
          <div className="min-w-0">
            {header}
            {topic.trade ? (
              <p className="mb-3 text-sm">
                <Link
                  href={`/trade/${topic.trade.listing_id}`}
                  className="text-accent hover:underline"
                >
                  查看价格与交易条款
                </Link>
              </p>
            ) : null}
            {floors}
          </div>
          <aside className="hidden xl:block">
            <div className="sticky top-24">
              <Timeline points={points} start={posts[0]?.floor} topicId={id} />
            </div>
          </aside>
        </AppShell>
      )}
      <BottomComposer
        mode={{ kind: "reply", topicId: id }}
        viewer={me ? headerViewer(me) : null}
        mentionable={mentionable}
        blocked={
          data.can_reply ? undefined : (
            <ReplyNotice signedIn={me !== null} locked={topic.flags.locked} here={here} />
          )
        }
        action={
          auction && !(auction instanceof ApiError) && auction.auction.state === "live" ? (
            <Button asChild size="sm" className="xl:hidden">
              <a href="#bid-amount">
                <Gavel size={16} aria-hidden />
                出价
              </a>
            </Button>
          ) : null
        }
      />
    </ComposerProvider>
  );
}
