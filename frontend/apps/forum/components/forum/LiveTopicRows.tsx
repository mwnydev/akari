import type { Schemas } from "@akari/api-client";
import type { UserCardView } from "../../lib/display";
import { topicRowView, tradeFacts } from "../../lib/topic-views";
import type { FeedRow } from "./TopicFeed";
import { TopicRow } from "./TopicTable";
import { TradeLine } from "./TradeLine";

/** 接口里的主题 → 服务端渲染好的列表行（交给 TopicFeed 做排序标签与筛选）。 */
export function liveFeedRows(
  topics: Schemas["TopicSummary"][],
  ctx: {
    user: (id: number) => UserCardView;
    /** 跨板块列表：按 forum_id 找板块，行里显示板块名 */
    forum?: (id: number) => Schemas["ForumView"] | undefined;
  },
): FeedRow[] {
  const now = new Date();
  return topics.map((t) => {
    const view = topicRowView(t, ctx.user, { forum: ctx.forum?.(t.forum_id), now });
    const trade = tradeFacts(t, ctx.user);
    return {
      id: t.id,
      kind: view.kind,
      pinned: view.pinned,
      node: <TopicRow key={t.id} topic={view} info={trade ? <TradeLine trade={trade} /> : null} />,
    };
  });
}
