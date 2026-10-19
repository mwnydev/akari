import type { Metadata } from "next";
import { BoardDirectory } from "../../../components/forum/BoardDirectory";
import { BottomComposer } from "../../../components/forum/BottomComposer";
import { ComposerProvider } from "../../../components/forum/ComposerContext";
import { NewTopicButton } from "../../../components/forum/NewTopicButton";
import { TopicFeed } from "../../../components/forum/TopicFeed";
import { MockShell } from "./_components/MockShell";
import { boards, hotTopics, topics } from "./_mock/data";
import { mockDirectory, mockRows } from "./_mock/views";

export const metadata: Metadata = { title: "首页" };

export default function DesignHomePage() {
  return (
    <ComposerProvider>
      <MockShell page="/design/v2" active="home">
        <h1 className="sr-only">Akari 论坛</h1>
        <TopicFeed
          lists={[
            { value: "latest", label: "最新", rows: mockRows(topics) },
            { value: "hot", label: "热门", rows: mockRows(hotTopics) },
          ]}
          extra={{
            value: "boards",
            label: "板块",
            node: <BoardDirectory boards={mockDirectory} />,
          }}
          action={<NewTopicButton />}
          empty={null}
        />
      </MockShell>
      <BottomComposer
        mode={{ kind: "topic", boards: boards.map((b) => ({ slug: b.slug, name: b.name })) }}
      />
    </ComposerProvider>
  );
}
