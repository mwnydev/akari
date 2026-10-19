import { describe, expect, it } from "vitest";
import { actionLabel, formatToken, userLabel } from "./format";
import { parseUserQuery } from "./users";

describe("format", () => {
  it("formats token amounts without floats", () => {
    expect(formatToken("45000000", 6)).toBe("45 USDT");
    expect(formatToken("12500001", 6)).toBe("12.500001 USDT");
    expect(formatToken("0", 6)).toBe("0 USDT");
    expect(formatToken("123456789012345678901234567890", 6)).toBe(
      "123456789012345678901234.56789 USDT",
    );
  });

  it("labels users and actions", () => {
    const names = new Map([[7, "alice"]]);
    expect(userLabel(7, names)).toBe("alice（#7）");
    expect(userLabel(8, names)).toBe("#8");
    expect(actionLabel("topic.hide")).toBe("隐藏主题");
    expect(actionLabel("x.unknown")).toBe("x.unknown");
  });
});

describe("parseUserQuery", () => {
  it("reads ids and username prefixes", () => {
    expect(parseUserQuery("#42")).toEqual({ id: 42 });
    expect(parseUserQuery(" 42 ")).toEqual({ id: 42 });
    expect(parseUserQuery("alice")).toEqual({ prefix: "alice" });
    expect(parseUserQuery("   ")).toBeNull();
  });
});
