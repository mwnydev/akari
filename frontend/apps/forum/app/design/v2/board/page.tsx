import type { Metadata } from "next";
import { board, tradeBoardTopics, type TopicSummary } from "../_mock/data";
import { BoardFeed, type FeedRow } from "../_components/BoardFeed";
import { BoardSquare } from "../_components/BoardMark";
import { ForumShell } from "../_components/ForumShell";
import { TopicRow } from "../_components/TopicRow";

export const metadata: Metadata = { title: "交易板块" };

function rows(list: TopicSummary[]): FeedRow[] {
  return list.map((t) => ({
    id: t.id,
    kind: t.trade ? "trade" : t.auction ? "auction" : "normal",
    pinned: t.pinned ?? false,
    node: <TopicRow key={t.id} topic={t} />,
  }));
}

export default function DesignBoardPage() {
  const b = board("trade");
  const pinned = tradeBoardTopics.filter((t) => t.pinned);
  const hot = [...tradeBoardTopics.filter((t) => !t.pinned)].sort((x, y) => y.views - x.views);
  return (
    <ForumShell page="/design/v2/board" active="trade">
      <header className="mb-4">
        <h1 className="flex items-center gap-2 text-2xl leading-snug font-semibold">
          <BoardSquare color={b.color} className="size-4" />
          {b.name}
        </h1>
        <p className="mt-1 text-fg-muted">{b.description}</p>
        <p className="amount mt-0.5 text-sm text-fg-muted">
          {b.topics} 个主题 · 今日 {b.today} · 版主 小林
        </p>
      </header>
      <BoardFeed latest={rows(tradeBoardTopics)} hot={rows([...pinned, ...hot])} />
    </ForumShell>
  );
}
