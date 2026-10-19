import type { Metadata } from "next";
import { BoardSquare } from "../../../../components/forum/BoardMark";
import { BottomComposer } from "../../../../components/forum/BottomComposer";
import { ComposerProvider } from "../../../../components/forum/ComposerContext";
import { NewTopicButton } from "../../../../components/forum/NewTopicButton";
import { TopicFeed } from "../../../../components/forum/TopicFeed";
import { MockShell } from "../_components/MockShell";
import { board, tradeBoardTopics } from "../_mock/data";
import { mockRows } from "../_mock/views";

export const metadata: Metadata = { title: "交易板块" };

export default function DesignBoardPage() {
  const b = board("trade");
  const pinned = tradeBoardTopics.filter((t) => t.pinned);
  const hot = tradeBoardTopics.filter((t) => !t.pinned).sort((x, y) => y.views - x.views);
  return (
    <ComposerProvider>
      <MockShell page="/design/v2/board" active="trade">
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
        <TopicFeed
          lists={[
            { value: "latest", label: "最新", rows: mockRows(tradeBoardTopics) },
            { value: "hot", label: "热门", rows: mockRows([...pinned, ...hot]) },
          ]}
          filters={["trade", "auction"]}
          action={<NewTopicButton />}
          empty={null}
        />
      </MockShell>
      <BottomComposer mode={{ kind: "topic", boards: [{ slug: b.slug, name: b.name }] }} />
    </ComposerProvider>
  );
}
