import { ApiError } from "@akari/api-client";
import { Button, Card, EmptyState } from "@akari/ui";
import { BadgeDollarSign } from "lucide-react";
import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";
import { cache } from "react";
import { ComposerLauncher } from "../../../components/composer/ComposerLauncher";
import { PageError } from "../../../components/PageError";
import { Pager } from "../../../components/Pager";
import { SiteNav } from "../../../components/SiteNav";
import { TopicRow } from "../../../components/TopicRow";
import { load, serverApi } from "../../../lib/api";
import { cardLookup } from "../../../lib/display";
import { boardHref, loginHref } from "../../../lib/links";
import { parsePositiveInt } from "../../../lib/pagination";
import { BOARD_MAX_PAGE } from "../../../lib/public-config";
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
  const q = boardQuery(await searchParams);
  const [data, me] = await Promise.all([loadBoard(slug, q.page, q.cursor), getMe()]);
  if (data instanceof ApiError) {
    if (data.status === 404) notFound();
    return (
      <>
        <SiteNav active={boardHref(slug)} />
        <PageError error={data} />
      </>
    );
  }
  const { forum } = data;
  const user = cardLookup(data.users);
  const empty = data.pinned.length + data.topics.length === 0;
  const older = data.next_cursor ? (
    <Button asChild size="sm" variant="secondary">
      <Link href={boardHref(forum.slug, 1, data.next_cursor)}>更早的主题</Link>
    </Button>
  ) : null;

  return (
    <>
      <SiteNav active={boardHref(forum.slug)} />
      <main className="mx-auto max-w-5xl space-y-4 px-4 py-5">
        <header className="flex flex-wrap items-end justify-between gap-3">
          <div>
            <h1 className="text-2xl font-semibold">{forum.name}</h1>
            <p className="text-fg-muted">{forum.description}</p>
          </div>
          {me ? (
            <Button asChild variant="secondary">
              <Link href={`${boardHref(forum.slug)}/listing`}>
                <BadgeDollarSign size={18} aria-hidden />
                发起交易帖
              </Link>
            </Button>
          ) : (
            <Button asChild>
              <Link href={loginHref(boardHref(forum.slug))}>登录后发帖</Link>
            </Button>
          )}
        </header>
        {me ? (
          <Card>
            <ComposerLauncher target={{ kind: "topic", forumSlug: forum.slug }} label="发主题" />
          </Card>
        ) : null}
        <Card className="p-0 sm:p-0">
          {empty && (q.page > 1 || q.cursor) ? (
            <EmptyState
              title="这一页没有主题"
              action={
                <Button asChild size="sm" variant="secondary">
                  <Link href={boardHref(forum.slug)}>回到第 1 页</Link>
                </Button>
              }
            />
          ) : empty ? (
            <EmptyState title="还没有主题" description="来发第一帖吧" />
          ) : (
            <ul className="divide-y divide-border">
              {[...data.pinned, ...data.topics].map((t) => (
                <TopicRow key={t.id} topic={t} user={user} />
              ))}
            </ul>
          )}
        </Card>
        {q.cursor ? (
          <nav aria-label="分页" className="flex justify-center gap-2 py-2">
            <Button asChild size="sm" variant="secondary">
              <Link href={boardHref(forum.slug)}>回到第 1 页</Link>
            </Button>
            {older}
          </nav>
        ) : (
          <Pager
            current={q.page}
            hasNext={data.next_page != null}
            href={(p) => boardHref(forum.slug, p)}
            after={older}
          />
        )}
      </main>
    </>
  );
}
