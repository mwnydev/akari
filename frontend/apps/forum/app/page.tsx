import { ApiError, type Schemas } from "@akari/api-client";
import { Card, CardTitle, EmptyState, ErrorState } from "@akari/ui";
import { ShieldCheck } from "lucide-react";
import Link from "next/link";
import { PageError } from "../components/PageError";
import { SiteNav } from "../components/SiteNav";
import { TopicRow } from "../components/TopicRow";
import { load, serverApi } from "../lib/api";
import { cardLookup } from "../lib/display";
import { forumTree, listForums, type ForumView } from "../lib/forums";

const LATEST = 20;

/**
 * 首页「最新主题」：后端还没有跨板块的最新列表，先并行取各板块第 1 页（不含置顶）再按最后回复时间合并。
 * 单个板块失败不影响其他板块。
 */
async function latestTopics(forums: ForumView[]) {
  const api = await serverApi();
  const pages = await Promise.all(
    forums.map((f) =>
      load(api.GET("/api/v1/forums/{slug}/topics", { params: { path: { slug: f.slug } } })),
    ),
  );
  const ok = pages.filter((p): p is Schemas["BoardResponse"] => !(p instanceof ApiError));
  const topics = ok
    .flatMap((p) => p.topics)
    .sort((a, b) => b.last_post_at.localeCompare(a.last_post_at))
    .slice(0, LATEST);
  return {
    topics,
    user: cardLookup(ok.flatMap((p) => p.users)),
    failed: ok.length < pages.length,
  };
}

export default async function Home() {
  const forums = await listForums();
  if (forums instanceof ApiError) {
    return (
      <>
        <SiteNav active="/" />
        <PageError error={forums} />
      </>
    );
  }
  const tree = forumTree(forums);
  const latest = await latestTopics(forums);
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
            {latest.topics.length ? (
              <ul className="divide-y divide-border">
                {latest.topics.map((t) => (
                  <TopicRow key={t.id} topic={t} user={latest.user} />
                ))}
              </ul>
            ) : latest.failed ? (
              <ErrorState message="最新主题加载失败，请稍后刷新" />
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
