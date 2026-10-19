import { describe, expect, it } from "vitest";
import { FORUM_ORIGIN } from "./domains";
import { forumReturnUrl } from "./return-url";

describe("forumReturnUrl", () => {
  it("keeps forum paths on the forum origin", () => {
    expect(forumReturnUrl("/t/103")).toBe(`${FORUM_ORIGIN}/t/103`);
    expect(forumReturnUrl("/t/103?page=2#floor-3")).toBe(`${FORUM_ORIGIN}/t/103?page=2#floor-3`);
  });

  it("falls back to the forum home for anything that could leave it", () => {
    for (const bad of [
      null,
      "",
      "https://evil.example/t/1",
      "//evil.example/t/1",
      "/\\evil.example",
      "javascript:alert(1)",
      "t/103",
      "/t/1 x",
    ]) {
      expect(forumReturnUrl(bad)).toBe(FORUM_ORIGIN);
    }
  });
});
