import { describe, expect, it } from "vitest";
import { clearDraft, loadDraft, saveDraft } from "./drafts";

function memoryStorage() {
  const m = new Map<string, string>();
  return {
    getItem: (k: string) => m.get(k) ?? null,
    setItem: (k: string, v: string) => void m.set(k, v),
    removeItem: (k: string) => void m.delete(k),
    raw: m,
  };
}

const doc = { type: "doc", content: [{ type: "paragraph" }] };

describe("drafts", () => {
  it("round-trips and clears a draft", () => {
    const s = memoryStorage();
    const draft = { title: "标题", doc, savedAt: Date.now() };
    expect(saveDraft("reply:1", draft, s)).toBe(true);
    expect(loadDraft("reply:1", s)).toEqual(draft);
    clearDraft("reply:1", s);
    expect(loadDraft("reply:1", s)).toBeNull();
  });

  it("ignores corrupt and expired drafts", () => {
    const s = memoryStorage();
    s.setItem("akari:draft:a", "{not json");
    s.setItem("akari:draft:b", JSON.stringify({ doc: null, savedAt: 1 }));
    s.setItem("akari:draft:c", JSON.stringify({ doc, savedAt: Date.now() - 31 * 86_400_000 }));
    expect(loadDraft("a", s)).toBeNull();
    expect(loadDraft("b", s)).toBeNull();
    expect(loadDraft("c", s)).toBeNull();
    expect(s.raw.has("akari:draft:c")).toBe(false);
  });

  it("survives a storage that throws", () => {
    const broken = {
      getItem: () => {
        throw new Error("denied");
      },
      setItem: () => {
        throw new Error("quota");
      },
      removeItem: () => {
        throw new Error("denied");
      },
    };
    expect(loadDraft("x", broken)).toBeNull();
    expect(saveDraft("x", { doc, savedAt: 1 }, broken)).toBe(false);
    expect(() => clearDraft("x", broken)).not.toThrow();
  });
});
