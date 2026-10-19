import type { Metadata } from "next";
import { topicFloors, topics, user, VIEWER_ID } from "../_mock/data";
import { BottomComposer } from "../_components/BottomComposer";
import { ComposerProvider } from "../_components/ComposerContext";
import { Floor, FloorStream } from "../_components/Floor";
import { ForumShell } from "../_components/ForumShell";
import { Timeline } from "../_components/Timeline";
import { TopicHeader } from "../_components/TopicHeader";

export const metadata: Metadata = { title: "普通帖" };

export default function DesignTopicPage() {
  const topic = topics.find((t) => t.id === 104);
  if (!topic) throw new Error("mock topic 104 missing");
  const floors = topicFloors;
  return (
    <ComposerProvider>
      <ForumShell
        page="/design/v2/topic"
        active={topic.board}
        className="xl:grid xl:grid-cols-[minmax(0,48rem)_9rem] xl:gap-10"
      >
        <div className="min-w-0">
          <TopicHeader topic={{ ...topic, replies: floors.length - 1 }} />
          <FloorStream label="楼层">
            {floors.map((f) => (
              <Floor key={f.floor} data={f} opAuthorId={topic.authorId} />
            ))}
          </FloorStream>
          <p className="py-6 text-center text-sm text-fg-muted">
            已经到底了 · 共 {floors.length} 层
          </p>
        </div>
        <aside className="hidden xl:block">
          <div className="sticky top-24">
            <Timeline posts={floors.map((f) => ({ floor: f.floor, time: f.time }))} />
          </div>
        </aside>
      </ForumShell>
      <BottomComposer viewer={user(VIEWER_ID).name} />
    </ComposerProvider>
  );
}
