import { describe, expect, it } from "vitest";
import { usernameProblem } from "./username";

describe("usernameProblem", () => {
  it("accepts CJK, ASCII letters, digits and underscores", () => {
    expect(usernameProblem(" 小明_abc9 ")).toBeNull();
    expect(usernameProblem("ab")).toBeNull();
    expect(usernameProblem("汉".repeat(20))).toBeNull();
  });

  it("rejects bad lengths, characters and all-digit names", () => {
    expect(usernameProblem("")).toBe("请输入用户名");
    expect(usernameProblem("a")).toMatch(/2–20/);
    expect(usernameProblem("a".repeat(21))).toMatch(/2–20/);
    expect(usernameProblem("12345")).toMatch(/纯数字/);
    expect(usernameProblem("a-b")).not.toBeNull();
    expect(usernameProblem("Ａbc")).not.toBeNull(); // 全角字母
  });
});
