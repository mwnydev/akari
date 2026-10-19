import type { Schemas } from "@akari/api-client";
import { can, type Permission } from "../../lib/staff";

export type TopicAction = Schemas["ModerationAction"];

export interface TopicActionItem {
  action: TopicAction;
  label: string;
  danger: boolean;
}

const NEEDS: Record<TopicAction, Permission> = {
  pin: "topic_moderate",
  unpin: "topic_moderate",
  feature: "topic_moderate",
  unfeature: "topic_moderate",
  lock: "topic_moderate",
  unlock: "topic_moderate",
  move: "topic_moderate",
  hide: "topic_hide",
  unhide: "topic_hide",
};

/** 主题当前状态下、访问者在本板块有权限做的操作（置顶 / 精华 / 锁帖 / 隐藏 / 移动）。 */
export function topicActions(
  staff: Schemas["StaffView"] | null | undefined,
  forumId: number,
  flags: Schemas["TopicFlags"],
  canMoveSomewhere: boolean,
): TopicActionItem[] {
  const all: TopicActionItem[] = [
    flags.pinned
      ? { action: "unpin", label: "取消置顶", danger: false }
      : { action: "pin", label: "置顶", danger: false },
    flags.featured
      ? { action: "unfeature", label: "取消精华", danger: false }
      : { action: "feature", label: "设为精华", danger: false },
    flags.locked
      ? { action: "unlock", label: "解除锁帖", danger: false }
      : { action: "lock", label: "锁帖（禁止回复）", danger: false },
    { action: "move", label: "移动到其他板块", danger: false },
    flags.hidden
      ? { action: "unhide", label: "恢复显示", danger: false }
      : { action: "hide", label: "隐藏主题", danger: true },
  ];
  return all.filter(
    (a) => can(staff, NEEDS[a.action], forumId) && (a.action !== "move" || canMoveSomewhere),
  );
}
