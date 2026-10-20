import { afterEach, describe, expect, it, vi } from "vitest";
import { dismissPrompt, promptDismissed } from "./username-prompt";

function fakeStorage(): Storage {
  const m = new Map<string, string>();
  return {
    get length() {
      return m.size;
    },
    clear: () => m.clear(),
    getItem: (k) => m.get(k) ?? null,
    key: (i) => [...m.keys()][i] ?? null,
    removeItem: (k) => void m.delete(k),
    setItem: (k, v) => void m.set(k, v),
  };
}

afterEach(() => vi.unstubAllGlobals());

describe("username prompt", () => {
  it("remembers the dismissal per account", () => {
    vi.stubGlobal("localStorage", fakeStorage());
    expect(promptDismissed(7)).toBe(false);
    dismissPrompt(7);
    expect(promptDismissed(7)).toBe(true);
    expect(promptDismissed(8)).toBe(false);
  });

  it("treats broken storage as not dismissed", () => {
    vi.stubGlobal("localStorage", {
      getItem: () => {
        throw new Error("denied");
      },
      setItem: () => {
        throw new Error("denied");
      },
    });
    expect(() => dismissPrompt(7)).not.toThrow();
    expect(promptDismissed(7)).toBe(false);
  });
});
