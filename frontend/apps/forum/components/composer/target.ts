// 编辑器的写入目标与草稿键。独立成轻量模块：启动按钮只需要它，不必提前加载 Tiptap。

export type ComposeTarget =
  | { kind: "topic"; forumSlug: string }
  | { kind: "reply"; topicId: number }
  | { kind: "edit"; postId: number; floor: number; title?: string | undefined };

export function draftKey(t: ComposeTarget): string {
  switch (t.kind) {
    case "topic":
      return `topic:${t.forumSlug}`;
    case "reply":
      return `reply:${t.topicId}`;
    case "edit":
      return `edit:${t.postId}`;
  }
}
