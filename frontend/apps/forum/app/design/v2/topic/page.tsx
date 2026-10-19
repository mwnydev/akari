import type { Metadata } from "next";
import { BottomComposer } from "../../../../components/forum/BottomComposer";
import { ComposerProvider } from "../../../../components/forum/ComposerContext";
import { FloorStream } from "../../../../components/forum/Floor";
import { Timeline } from "../../../../components/forum/Timeline";
import { TopicHeader } from "../../../../components/forum/TopicHeader";
import { TopicModMenu } from "../../../../components/moderation/TopicModMenu";
import { MockFloor } from "../_components/MockFloor";
import { MockShell } from "../_components/MockShell";
import { topicFloors, topics, user, VIEWER_ID } from "../_mock/data";
import { mockForums, mockHeader, mockStaff } from "../_mock/views";

export const metadata: Metadata = { title: "普通帖" };

export default function DesignTopicPage() {
  const topic = topics.find((t) => t.id === 104);
  if (!topic) throw new Error("mock topic 104 missing");
  const floors = topicFloors;
  return (
    <ComposerProvider>
      <MockShell
        page="/design/v2/topic"
        active={topic.board}
        className="xl:grid xl:grid-cols-[minmax(0,48rem)_9rem] xl:gap-10"
      >
        <div className="min-w-0">
          <TopicHeader
            topic={{ ...mockHeader(topic), replies: floors.length - 1 }}
            actions={
              mockStaff ? (
                <TopicModMenu
                  topicId={topic.id}
                  forumId={1}
                  flags={{ pinned: false, featured: false, locked: false, hidden: false }}
                  forums={mockForums}
                  staff={mockStaff}
                />
              ) : null
            }
          />
          <FloorStream label="楼层">
            {floors.map((f) => (
              <MockFloor
                key={f.floor}
                page="/design/v2/topic"
                data={f}
                opAuthorId={topic.authorId}
              />
            ))}
          </FloorStream>
          <p className="py-6 text-center text-sm text-fg-muted">
            已经到底了 · 共 {floors.length} 层
          </p>
        </div>
        <aside className="hidden xl:block">
          <div className="sticky top-24">
            <Timeline points={floors.map((f) => ({ floor: f.floor, time: f.time }))} />
          </div>
        </aside>
      </MockShell>
      <BottomComposer
        mode={{ kind: "reply", topicId: topic.id }}
        viewer={{ name: user(VIEWER_ID).name }}
      />
    </ComposerProvider>
  );
}
