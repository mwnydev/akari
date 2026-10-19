import { Badge, Button, Card, EmptyState } from "@akari/ui";
import { PenLine } from "lucide-react";
import { notFound } from "next/navigation";
import { SiteNav } from "../../../components/SiteNav";
import { TopicRow } from "../../../components/TopicRow";
import { boards, topics } from "../../../lib/mock";

export default async function BoardPage({ params }: { params: Promise<{ slug: string }> }) {
  const { slug } = await params;
  const board = boards.find((b) => b.slug === slug);
  if (!board) notFound();
  // 置顶帖只在第 1 页顶部单独展示（plan 第 8 节）
  const pinned = topics.filter((t) => t.pinned);
  const list = topics.filter(
    (t) => !t.pinned && (t.board === slug || board.trade === Boolean(t.trade)),
  );
  return (
    <>
      <SiteNav active={`/f/${slug}`} />
      <main className="mx-auto max-w-5xl space-y-4 px-4 py-5">
        <header className="flex flex-wrap items-end justify-between gap-3">
          <div>
            <h1 className="flex items-center gap-2 text-2xl font-semibold">
              {board.name}
              {board.trade ? <Badge tone="warning">担保交易</Badge> : null}
            </h1>
            <p className="text-fg-muted">{board.description}</p>
          </div>
          <Button>
            <PenLine size={18} aria-hidden />
            {board.trade ? "发交易帖" : "发主题"}
          </Button>
        </header>
        <Card className="p-0 sm:p-0">
          {pinned.length + list.length === 0 ? (
            <EmptyState title="还没有主题" description="来发第一帖吧" />
          ) : (
            <ul className="divide-y divide-border">
              {pinned.map((t) => (
                <TopicRow key={t.id} t={t} />
              ))}
              {list.map((t) => (
                <TopicRow key={t.id} t={t} />
              ))}
            </ul>
          )}
        </Card>
        <div className="flex justify-center">
          <Button variant="secondary">加载更多</Button>
        </div>
      </main>
    </>
  );
}
