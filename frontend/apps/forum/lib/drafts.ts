// 草稿自动保存到 localStorage（plan 第 4 节）。读写都包在 try/catch 里：
// 隐私模式、存储已满或被禁用时草稿功能静默失效，不影响发帖。
import type { JSONContent } from "@tiptap/react";

export interface Draft {
  title?: string;
  doc: JSONContent;
  savedAt: number;
}

const PREFIX = "akari:draft:";
/** 超过 30 天的草稿视为放弃，读取时顺手清掉。 */
const MAX_AGE_MS = 30 * 86_400_000;

type Storage = Pick<globalThis.Storage, "getItem" | "setItem" | "removeItem">;

function browserStorage(): Storage | null {
  try {
    return globalThis.localStorage ?? null;
  } catch {
    return null;
  }
}

export function loadDraft(key: string, storage = browserStorage()): Draft | null {
  try {
    const raw = storage?.getItem(PREFIX + key);
    if (!raw) return null;
    const d = JSON.parse(raw) as Partial<Draft>;
    if (typeof d.savedAt !== "number" || typeof d.doc !== "object" || d.doc === null) return null;
    if (Date.now() - d.savedAt > MAX_AGE_MS) {
      storage?.removeItem(PREFIX + key);
      return null;
    }
    return d as Draft;
  } catch {
    return null;
  }
}

export function saveDraft(key: string, draft: Draft, storage = browserStorage()): boolean {
  try {
    storage?.setItem(PREFIX + key, JSON.stringify(draft));
    return storage !== null;
  } catch {
    return false;
  }
}

export function clearDraft(key: string, storage = browserStorage()): void {
  try {
    storage?.removeItem(PREFIX + key);
  } catch {
    // 同上：存储不可用时没有草稿可清
  }
}
