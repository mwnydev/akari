import { JSDOM } from "jsdom";
import { describe, expect, it } from "vitest";
import { NO_REPUTATION, type UserCardView } from "./display";
import { linkMentions, mentionCandidates, mentionQuery } from "./mentions";

function must<T>(v: T | null | undefined): T {
  if (v == null) throw new Error("missing");
  return v;
}

describe("mentionQuery", () => {
  it("accepts what a username can start with", () => {
    expect(mentionQuery(" 小林 ")).toBe("小林");
    expect(mentionQuery("ab_9")).toBe("ab_9");
  });

  it("skips the request when no username could match", () => {
    expect(mentionQuery("")).toBeNull();
    expect(mentionQuery("  ")).toBeNull();
    expect(mentionQuery("a b")).toBeNull();
    expect(mentionQuery("a%")).toBeNull();
    expect(mentionQuery("абв")).toBeNull(); // 西里尔字母不能出现在用户名里
    expect(mentionQuery("x".repeat(21))).toBeNull();
    expect(mentionQuery("汉".repeat(20))).toBe("汉".repeat(20));
  });
});

describe("linkMentions", () => {
  const render = (html: string) => {
    const { document } = new JSDOM(`<div id="r">${html}</div>`).window;
    const root = must(document.getElementById("r"));
    linkMentions(root);
    return root.innerHTML;
  };

  it("links every numeric mention, keeping its text", () => {
    expect(
      render(
        '<span data-type="mention" data-id="3">@a</span> and <span data-type="mention" data-id="12" data-label="b">@b</span>',
      ),
    ).toBe(
      '<a href="/u/3" data-type="mention">@a</a> and <a href="/u/12" data-type="mention">@b</a>',
    );
  });

  it("leaves malformed mentions and other spans alone", () => {
    const html =
      '<span data-type="mention">@nobody</span><span data-type="mention" data-id="../x">@x</span><span>plain</span>';
    expect(render(html)).toBe(html);
  });
});

describe("mentionCandidates", () => {
  const card = (id: number, username: string | null): UserCardView => ({
    id,
    username,
    level: 1,
    reputation: NO_REPUTATION,
  });

  it("puts people from this page first, then search results, without duplicates", () => {
    const local = [card(1, "Momo"), card(2, "lam"), card(3, null)];
    const remote = [
      { id: 1, username: "Momo", level: 2 },
      { id: 4, username: "momoko", level: 1 },
    ];
    expect(mentionCandidates(local, remote, "mo")).toEqual([
      { id: 1, name: "Momo" },
      { id: 4, name: "momoko" },
    ]);
  });

  it("matches users without a username by their display name and caps the list", () => {
    const local = Array.from({ length: 12 }, (_, i) => card(i + 1, null));
    expect(mentionCandidates(local, [], "")).toHaveLength(8);
    expect(mentionCandidates(local, [], "用户 1").map((c) => c.id)).toEqual([1, 10, 11, 12]);
  });
});
