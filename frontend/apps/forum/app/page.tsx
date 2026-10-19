import { ApiError } from "@akari/api-client";
import { Button, EmptyState, ErrorState } from "@akari/ui";
import Link from "next/link";
import { AppShell } from "../components/forum/AppShell";
import { BoardDirectory } from "../components/forum/BoardDirectory";
import { BottomComposer } from "../components/forum/BottomComposer";
import { ComposerProvider } from "../components/forum/ComposerContext";
import { liveFeedRows } from "../components/forum/LiveTopicRows";
import { NewTopicButton } from "../components/forum/NewTopicButton";
import { TopicFeed, type FeedList } from "../components/forum/TopicFeed";
import { PageError } from "../components/PageError";
import { load, serverApi } from "../lib/api";
import { boardColor } from "../lib/board-color";
import { cardLookup } from "../lib/display";
import { forumTree, listForums, type ForumView } from "../lib/forums";
import { boardHref, loginHref } from "../lib/links";
import { needsPostCaptcha } from "../lib/public-config";
import { getMe } from "../lib/session";

type Search = Promise<Record<string, string | string[] | undefined>>;

const TABS = ["latest", "hot", "boards"];

/** 发新帖的板块候选：发帖门槛低的排前面（默认选中第一个）。 */
function postableBoards(forums: ForumView[]) {
  return [...forums]
    .sort((a, b) => a.post_min_level - b.post_min_level || a.position - b.position)
    .map((f) => ({ slug: f.slug, name: f.name }));
}

/**
 * 首页：跨板块最新主题（后端按最后回复时间，只含访问者看得到的板块）、热门（后端 `sort=hot`，排名每 5 分钟更新）、
 * 板块目录三个标签页。`?cursor=` 翻到更早的最新主题，`?hot=` 翻热门的下一页；`?tab=hot` / `?tab=boards`
 * 直接打开对应标签页（侧栏「热门」）。
 */
export default async function Home({ searchParams }: { searchParams: Search }) {
  const sp = await searchParams;
  const cursor = typeof sp.cursor === "string" && sp.cursor ? sp.cursor : undefined;
  const hotCursor = typeof sp.hot === "string" && sp.hot ? sp.hot : undefined;
  const tab = hotCursor
    ? "hot"
    : typeof sp.tab === "string" && TABS.includes(sp.tab)
      ? sp.tab
      : "latest";
  const api = await serverApi();
  const [forums, latest, hot, me] = await Promise.all([
    listForums(),
    load(api.GET("/api/v1/topics/latest", { params: { query: cursor ? { cursor } : {} } })),
    load(
      api.GET("/api/v1/topics/latest", {
        params: { query: { sort: "hot", ...(hotCursor ? { cursor: hotCursor } : {}) } },
      }),
    ),
    getMe(),
  ]);
  if (forums instanceof ApiError) {
    return (
      <AppShell active="home">
        <PageError error={forums} />
      </AppShell>
    );
  }
  const byId = new Map(forums.map((f) => [f.id, f]));
  const directory = forumTree(forums).map((b) => ({
    slug: b.slug,
    name: b.name,
    color: boardColor(b),
    href: boardHref(b.slug),
    description: b.description,
    children: b.children.map((c) => ({ slug: c.slug, name: c.name, href: boardHref(c.slug) })),
  }));

  let lists: FeedList[] | null = null;
  if (!(latest instanceof ApiError)) {
    const ctx = { user: cardLookup(latest.users), forum: (id: number) => byId.get(id) };
    lists = [
      {
        value: "latest",
        label: "最新",
        rows: liveFeedRows(latest.topics, ctx),
        footer: latest.next_cursor ? (
          <p className="py-3 text-center text-sm">
            <Link
              href={`/?cursor=${encodeURIComponent(latest.next_cursor)}`}
              className="text-accent hover:underline"
            >
              更早的主题
            </Link>
          </p>
        ) : null,
      },
    ];
    if (!(hot instanceof ApiError)) {
      const hotCtx = { user: cardLookup(hot.users), forum: (id: number) => byId.get(id) };
      lists.push({
        value: "hot",
        label: "热门",
        rows: liveFeedRows(hot.topics, hotCtx),
        footer: hot.next_cursor ? (
          <p className="py-3 text-center text-sm">
            <Link
              href={`/?hot=${encodeURIComponent(hot.next_cursor)}`}
              className="text-accent hover:underline"
            >
              更多热门主题
            </Link>
          </p>
        ) : null,
      });
    }
  }

  return (
    <ComposerProvider captcha={needsPostCaptcha(me)}>
      <AppShell active={tab === "hot" ? "hot" : "home"}>
        <h1 className="sr-only">Akari 论坛</h1>
        {lists ? (
          <TopicFeed
            lists={lists}
            extra={{ value: "boards", label: "板块", node: <BoardDirectory boards={directory} /> }}
            defaultTab={tab}
            action={
              me ? (
                <NewTopicButton />
              ) : (
                <Button asChild size="sm">
                  <Link href={loginHref("/")}>登录后发帖</Link>
                </Button>
              )
            }
            empty={<EmptyState title="还没有主题" description="去板块里发第一帖吧" />}
          />
        ) : (
          <>
            <ErrorState message="最新主题加载失败，请稍后刷新" />
            <BoardDirectory boards={directory} />
          </>
        )}
      </AppShell>
      {me ? <BottomComposer mode={{ kind: "topic", boards: postableBoards(forums) }} /> : null}
    </ComposerProvider>
  );
}
