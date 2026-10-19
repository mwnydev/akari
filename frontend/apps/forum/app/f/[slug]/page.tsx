import { ApiError } from "@akari/api-client";
import { Button, EmptyState } from "@akari/ui";
import { BadgeDollarSign } from "lucide-react";
import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";
import { cache } from "react";
import { AppShell } from "../../../components/forum/AppShell";
import { BoardSquare } from "../../../components/forum/BoardMark";
import { BottomComposer } from "../../../components/forum/BottomComposer";
import { ComposerProvider } from "../../../components/forum/ComposerContext";
import { liveFeedRows } from "../../../components/forum/LiveTopicRows";
import { NewTopicButton } from "../../../components/forum/NewTopicButton";
import { TopicFeed } from "../../../components/forum/TopicFeed";
import { PageError } from "../../../components/PageError";
import { Pager } from "../../../components/Pager";
import { load, serverApi } from "../../../lib/api";
import { boardColor } from "../../../lib/board-color";
import { cardLookup } from "../../../lib/display";
import { boardHref, loginHref } from "../../../lib/links";
import { parsePositiveInt } from "../../../lib/pagination";
import { BOARD_MAX_PAGE, needsPostCaptcha } from "../../../lib/public-config";
import { getMe } from "../../../lib/session";

type Params = Promise<{ slug: string }>;
type Search = Promise<Record<string, string | string[] | undefined>>;

interface BoardQuery {
  page: number;
  cursor: string | undefined;
}

/** 前 200 页用 ?page=N；更深用 ?cursor=（取自上一页的 next_cursor，plan 第 8 节）。 */
function boardQuery(sp: Awaited<Search>): BoardQuery {
  const cursor = typeof sp.cursor === "string" && sp.cursor ? sp.cursor : undefined;
  return { page: Math.min(parsePositiveInt(sp.page), BOARD_MAX_PAGE), cursor };
}

const loadBoard = cache(async (slug: string, page: number, cursor: string | undefined) =>
  load(
    (await serverApi()).GET("/api/v1/forums/{slug}/topics", {
      params: { path: { slug }, query: cursor ? { cursor } : { page } },
    }),
  ),
);

/** 板块热门（后端 `sort=hot`，不含置顶帖；排名每 5 分钟更新），`?hot=` 翻下一页。 */
const loadHot = async (slug: string, cursor: string | undefined) =>
  load(
    (await serverApi()).GET("/api/v1/forums/{slug}/topics", {
      params: { path: { slug }, query: { sort: "hot", ...(cursor ? { cursor } : {}) } },
    }),
  );

export async function generateMetadata({
  params,
  searchParams,
}: {
  params: Params;
  searchParams: Search;
}): Promise<Metadata> {
  const { slug } = await params;
  const q = boardQuery(await searchParams);
  const data = await loadBoard(slug, q.page, q.cursor);
  return data instanceof ApiError
    ? {}
    : { title: data.forum.name, description: data.forum.description };
}

export default async function BoardPage({
  params,
  searchParams,
}: {
  params: Params;
  searchParams: Search;
}) {
  const { slug } = await params;
  const sp = await searchParams;
  const q = boardQuery(sp);
  const hotCursor = typeof sp.hot === "string" && sp.hot ? sp.hot : undefined;
  const [data, hot, me] = await Promise.all([
    loadBoard(slug, q.page, q.cursor),
    loadHot(slug, hotCursor),
    getMe(),
  ]);
  if (data instanceof ApiError) {
    if (data.status === 404) notFound();
    return (
      <AppShell active={slug}>
        <PageError error={data} />
      </AppShell>
    );
  }
  const { forum } = data;
  const topics = [...data.pinned, ...data.topics];
  const ctx = { user: cardLookup(data.users) };
  const older = data.next_cursor ? (
    <Button asChild size="sm" variant="secondary">
      <Link href={boardHref(forum.slug, 1, data.next_cursor)}>更早的主题</Link>
    </Button>
  ) : null;
  const pager = q.cursor ? (
    <nav aria-label="分页" className="flex justify-center gap-2 py-3">
      <Button asChild size="sm" variant="secondary">
        <Link href={boardHref(forum.slug)}>回到第 1 页</Link>
      </Button>
      {older}
    </nav>
  ) : (
    <div className="py-3">
      <Pager
        current={q.page}
        hasNext={data.next_page != null}
        href={(p) => boardHref(forum.slug, p)}
        after={older}
      />
    </div>
  );
  const deep = q.page > 1 || q.cursor !== undefined;

  return (
    <ComposerProvider captcha={needsPostCaptcha(me)}>
      <AppShell active={forum.slug}>
        <header className="mb-4">
          <h1 className="flex items-center gap-2 text-2xl leading-snug font-semibold">
            <BoardSquare color={boardColor(forum)} className="size-4" />
            {forum.name}
          </h1>
          <p className="mt-1 text-fg-muted">{forum.description}</p>
        </header>
        <TopicFeed
          lists={[
            { value: "latest", label: "最新", rows: liveFeedRows(topics, ctx), footer: pager },
            ...(hot instanceof ApiError
              ? []
              : [
                  {
                    value: "hot",
                    label: "热门",
                    rows: liveFeedRows(hot.topics, { user: cardLookup(hot.users) }),
                    footer: hot.next_cursor ? (
                      <p className="py-3 text-center text-sm">
                        <Link
                          href={`${boardHref(forum.slug)}?hot=${encodeURIComponent(hot.next_cursor)}`}
                          className="text-accent hover:underline"
                        >
                          更多热门主题
                        </Link>
                      </p>
                    ) : null,
                  },
                ]),
          ]}
          defaultTab={hotCursor ? "hot" : undefined}
          filters={forum.allow_trade ? ["trade"] : []}
          action={
            me ? (
              <>
                {forum.allow_trade ? (
                  <Button asChild size="sm" variant="secondary">
                    <Link href={`${boardHref(forum.slug)}/listing`}>
                      <BadgeDollarSign size={16} aria-hidden />
                      发交易帖
                    </Link>
                  </Button>
                ) : null}
                <NewTopicButton />
              </>
            ) : (
              <Button asChild size="sm">
                <Link href={loginHref(boardHref(forum.slug))}>登录后发帖</Link>
              </Button>
            )
          }
          empty={
            deep ? (
              <EmptyState
                title="这一页没有主题"
                action={
                  <Button asChild size="sm" variant="secondary">
                    <Link href={boardHref(forum.slug)}>回到第 1 页</Link>
                  </Button>
                }
              />
            ) : (
              <EmptyState title="还没有主题" description="来发第一帖吧" />
            )
          }
        />
      </AppShell>
      {me ? (
        <BottomComposer
          mode={{ kind: "topic", boards: [{ slug: forum.slug, name: forum.name }] }}
        />
      ) : null}
    </ComposerProvider>
  );
}
