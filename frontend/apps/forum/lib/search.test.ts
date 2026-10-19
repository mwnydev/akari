import { describe, expect, it } from "vitest";
import {
  MAX_QUERY_CHARS,
  normalizeQuery,
  parseSearchParams,
  searchHref,
  suggestQuery,
} from "./search";

describe("search params", () => {
  it("parses and defaults", () => {
    expect(parseSearchParams({ q: "  出售 netflix ", type: "topic", board: "trade" })).toEqual({
      q: "出售 netflix",
      type: "topic",
      board: "trade",
      cursor: undefined,
    });
    expect(parseSearchParams({ q: ["a", "b"], type: "evil", board: "", cursor: "9.20" })).toEqual({
      q: "a",
      type: "post",
      board: undefined,
      cursor: "9.20",
    });
    expect(parseSearchParams({}).q).toBe("");
  });

  it("truncates by characters, not code units", () => {
    const q = normalizeQuery("😀".repeat(100));
    expect(Array.from(q)).toHaveLength(MAX_QUERY_CHARS);
  });

  it("builds hrefs without defaults and encodes everything", () => {
    expect(searchHref({ q: "出售 & 收购" })).toBe(
      "/search?q=%E5%87%BA%E5%94%AE+%26+%E6%94%B6%E8%B4%AD",
    );
    expect(searchHref({ q: "x", type: "user", board: "a/b", cursor: "1.20" })).toBe(
      "/search?q=x&type=user&board=a%2Fb&cursor=1.20",
    );
    expect(searchHref({ q: "x", type: "post" })).toBe("/search?q=x");
  });

  it("only suggests from two characters on", () => {
    expect(suggestQuery(" 号 ")).toBeNull();
    expect(suggestQuery("会员")).toBe("会员");
    expect(suggestQuery("   ")).toBeNull();
  });
});
