import { ApiError } from "@akari/api-client";
import { Button, Input } from "@akari/ui";
import type { Metadata } from "next";
import Link from "next/link";
import { notFound, redirect } from "next/navigation";
import { cache } from "react";
import { AppShell } from "../../../components/forum/AppShell";
import { BottomComposer } from "../../../components/forum/BottomComposer";
import { ComposerProvider } from "../../../components/forum/ComposerContext";
import { FloorStream } from "../../../components/forum/Floor";
import { LiveFloors } from "../../../components/forum/LiveFloor";
import { ReplyNotice } from "../../../components/forum/ReplyNotice";
import { Timeline } from "../../../components/forum/Timeline";
import { TopicHeader } from "../../../components/forum/TopicHeader";
import { PageError } from "../../../components/PageError";
import { Pager } from "../../../components/Pager";
import { TopicModMenu } from "../../../components/moderation/TopicModMenu";
import { load, serverApi } from "../../../lib/api";
import { formatActivity } from "../../../lib/display";
import { listForums } from "../../../lib/forums";
import { topicHref } from "../../../lib/links";
import { headerViewer } from "../../../lib/nav";
import { floorPage, parsePositiveInt } from "../../../lib/pagination";
import { needsPostCaptcha } from "../../../lib/public-config";
import { getMe } from "../../../lib/session";
import { topicHeaderView } from "../../../lib/topic-views";

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

  return (
    <ComposerProvider captcha={needsPostCaptcha(me)}>
      <AppShell
        active={forum.slug}
        className="xl:grid xl:grid-cols-[minmax(0,48rem)_9rem] xl:gap-10"
      >
        <div className="min-w-0">
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
          <FloorStream label="楼层">
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
        </div>
        <aside className="hidden xl:block">
          <div className="sticky top-24">
            <Timeline points={points} start={posts[0]?.floor} topicId={id} />
          </div>
        </aside>
      </AppShell>
      <BottomComposer
        mode={{ kind: "reply", topicId: id }}
        viewer={me ? headerViewer(me) : null}
        mentionable={mentionable}
        blocked={
          data.can_reply ? undefined : (
            <ReplyNotice
              signedIn={me !== null}
              locked={topic.flags.locked}
              here={topicHref(id, page)}
            />
          )
        }
      />
    </ComposerProvider>
  );
}
