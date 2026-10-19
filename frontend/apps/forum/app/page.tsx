import { ApiError } from "@akari/api-client";
import { Card, CardTitle, EmptyState, ErrorState } from "@akari/ui";
import { ShieldCheck } from "lucide-react";
import Link from "next/link";
import { PageError } from "../components/PageError";
import { SiteNav } from "../components/SiteNav";
import { TopicRow } from "../components/TopicRow";
import { load, serverApi } from "../lib/api";
import { cardLookup } from "../lib/display";
import { forumTree, listForums } from "../lib/forums";

type Search = Promise<Record<string, string | string[] | undefined>>;

/** 首页：跨板块最新主题（后端按最后回复时间，只含访问者看得到的板块）+ 板块目录。`?cursor=` 翻到更早的。 */
export default async function Home({ searchParams }: { searchParams: Search }) {
  const raw = (await searchParams).cursor;
  const cursor = typeof raw === "string" && raw ? raw : undefined;
  const api = await serverApi();
  const [forums, latest] = await Promise.all([
    listForums(),
    load(api.GET("/api/v1/topics/latest", { params: { query: cursor ? { cursor } : {} } })),
  ]);
  if (forums instanceof ApiError) {
    return (
      <>
        <SiteNav active="/" />
        <PageError error={forums} />
      </>
    );
  }
  const tree = forumTree(forums);
  const user = latest instanceof ApiError ? null : cardLookup(latest.users);
  return (
    <>
      <SiteNav active="/" />
      <main className="mx-auto grid max-w-5xl gap-4 px-4 py-5 lg:grid-cols-[1fr_18rem]">
        <section className="min-w-0 space-y-4">
          <Card className="flex items-start gap-3 bg-accent-soft">
            <ShieldCheck className="mt-0.5 shrink-0 text-accent" aria-hidden />
            <div>
              <p className="font-medium">链上担保，平台转不走你的钱</p>
              <p className="text-sm text-fg-muted">
                USDT 托管在 Arbitrum One 合约里，确认收货才放款；gas 平台代付，钱包无需 ETH。
              </p>
            </div>
          </Card>
          <Card className="p-0 sm:p-0">
            <CardTitle className="px-4 pt-4">最新主题</CardTitle>
            {latest instanceof ApiError || !user ? (
              <ErrorState message="最新主题加载失败，请稍后刷新" />
            ) : latest.topics.length ? (
              <>
                <ul className="divide-y divide-border" aria-label="最新主题">
                  {latest.topics.map((t) => (
                    <TopicRow key={t.id} topic={t} user={user} />
                  ))}
                </ul>
                {latest.next_cursor ? (
                  <p className="px-4 py-3 text-center text-sm">
                    <Link
                      href={`/?cursor=${encodeURIComponent(latest.next_cursor)}`}
                      className="text-accent hover:underline"
                    >
                      更早的主题
                    </Link>
                  </p>
                ) : null}
              </>
            ) : (
              <EmptyState title="还没有主题" description="去板块里发第一帖吧" />
            )}
          </Card>
        </section>
        <aside className="min-w-0">
          <Card className="p-0 sm:p-0">
            <CardTitle className="px-4 pt-4">板块</CardTitle>
            {tree.length ? (
              <ul className="divide-y divide-border">
                {tree.map((b) => (
                  <li key={b.id} className="px-4 py-3">
                    <Link href={`/f/${b.slug}`} className="block hover:text-accent">
                      <span className="font-medium">{b.name}</span>
                      <span className="block text-sm text-fg-muted">{b.description}</span>
                    </Link>
                    {b.children.length ? (
                      <ul className="mt-1 flex flex-wrap gap-x-3 text-sm">
                        {b.children.map((c) => (
                          <li key={c.id}>
                            <Link href={`/f/${c.slug}`} className="text-accent hover:underline">
                              {c.name}
                            </Link>
                          </li>
                        ))}
                      </ul>
                    ) : null}
                  </li>
                ))}
              </ul>
            ) : (
              <EmptyState title="暂无可访问的板块" />
            )}
          </Card>
        </aside>
      </main>
    </>
  );
}
