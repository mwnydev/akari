// @提及：编辑器里的候选，与正文里提及节点的显示。
// Tiptap 提及节点渲染为 <span data-type="mention" data-id="用户 ID" data-label="名字">@名字</span>，
// 后端只认 data-id（据此发通知），所以显示时链接也只按 ID 生成，名字只当文字显示。

import type { Schemas } from "@akari/api-client";
import { userName, type UserCardView } from "./display";
import { USERNAME_CHARS, USERNAME_MAX_CHARS } from "./username";

export interface MentionCandidate {
  id: number;
  name: string;
}

const MAX_SUGGESTIONS = 8;

/** @ 之后输入的内容 → 用户搜索的前缀；空的或含用户名不可能有的字符返回 null（不必请求）。 */
export function mentionQuery(raw: string): string | null {
  const q = raw.trim();
  if (!q || Array.from(q).length > USERNAME_MAX_CHARS || !USERNAME_CHARS.test(q)) return null;
  return q;
}

/**
 * 候选列表：本页出现过的人里名字包含输入的排前面（回复楼里的人最常被 @），再接用户名搜索结果；按 ID 去重，最多 8 个。
 */
export function mentionCandidates(
  local: readonly UserCardView[],
  remote: readonly Schemas["MentionView"][],
  query: string,
): MentionCandidate[] {
  const q = query.trim().toLowerCase();
  const out: MentionCandidate[] = [];
  const seen = new Set<number>();
  const push = (id: number, name: string) => {
    if (seen.has(id) || out.length >= MAX_SUGGESTIONS) return;
    seen.add(id);
    out.push({ id, name });
  };
  for (const u of local) if (userName(u).toLowerCase().includes(q)) push(u.id, userName(u));
  for (const u of remote) push(u.id, u.username);
  return out;
}

/** 提及链接：按用户 ID 指向个人主页（`/u/{id}` 由后端按 ID 解析）。 */
export function mentionHref(id: string): string {
  return `/u/${id}`;
}

/**
 * 把已过滤的正文里的提及节点换成个人主页链接。只处理 data-id 为纯数字的节点；
 * 文字取节点原有的文字内容（textContent，不会解析成 HTML）。
 */
export function linkMentions(root: ParentNode & Node): void {
  const doc = root.ownerDocument ?? (root as unknown as Document);
  for (const span of Array.from(root.querySelectorAll('span[data-type="mention"]'))) {
    const id = span.getAttribute("data-id");
    if (!id || !/^\d{1,18}$/.test(id)) continue;
    const a = doc.createElement("a");
    a.setAttribute("href", mentionHref(id));
    a.setAttribute("data-type", "mention");
    a.textContent = span.textContent;
    span.replaceWith(a);
  }
}
