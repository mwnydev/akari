import { describe, expect, it } from "vitest";
import { avatarInitial } from "./initial";

describe("avatarInitial", () => {
  it("uppercases Latin letters", () => {
    expect(avatarInitial("alice")).toBe("A");
    expect(avatarInitial("Alice")).toBe("A");
    expect(avatarInitial("éclair")).toBe("É");
  });

  it("keeps CJK, digits and emoji unchanged", () => {
    expect(avatarInitial("小林")).toBe("小");
    expect(avatarInitial("用户 7")).toBe("用");
    expect(avatarInitial("7up")).toBe("7");
    expect(avatarInitial("😀x")).toBe("😀");
  });

  it("falls back to ? for empty names", () => {
    expect(avatarInitial("")).toBe("?");
    expect(avatarInitial("  ")).toBe("?");
  });
});
