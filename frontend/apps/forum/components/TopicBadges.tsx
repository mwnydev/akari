import type { Schemas } from "@akari/api-client";
import { Badge } from "@akari/ui";

/** TopicSummary.kind：0 普通、1 交易、2 争议。 */
const TOPIC_KIND_TRADE = 1;

/** 主题标题前的状态徽标：置顶 / 精华 / 交易 / 锁定 / 隐藏（隐藏只有管理人员看得到）。 */
export function TopicBadges({ topic }: { topic: Schemas["TopicSummary"] }) {
  const { pinned, featured, locked, hidden } = topic.flags;
  const badge = "mr-1.5 align-[2px]";
  return (
    <>
      {pinned ? (
        <Badge tone="accent" className={badge}>
          置顶
        </Badge>
      ) : null}
      {featured ? (
        <Badge tone="success" className={badge}>
          精华
        </Badge>
      ) : null}
      {topic.kind === TOPIC_KIND_TRADE ? (
        <Badge tone="warning" className={badge}>
          担保
        </Badge>
      ) : null}
      {locked ? (
        <Badge className={badge} aria-label="已锁定">
          锁定
        </Badge>
      ) : null}
      {hidden ? (
        <Badge tone="danger" className={badge}>
          已隐藏
        </Badge>
      ) : null}
    </>
  );
}
