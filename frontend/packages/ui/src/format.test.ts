import { describe, expect, it } from "vitest";
import { formatUsdt, parseUsdt } from "./format";

describe("formatUsdt", () => {
  it("formats with thousands separators, ≥ 2 decimals, trailing zeros trimmed", () => {
    expect(formatUsdt("0")).toBe("0.00");
    expect(formatUsdt("1")).toBe("0.000001");
    expect(formatUsdt(1_000_000n)).toBe("1.00");
    expect(formatUsdt("124124072")).toBe("124.124072");
    expect(formatUsdt("1234567890123456")).toBe("1,234,567,890.123456");
    expect(formatUsdt("-50000")).toBe("-0.05");
  });

  it("is exact beyond float precision", () => {
    expect(formatUsdt("9007199254740993000001")).toBe("9,007,199,254,740,993.000001");
  });

  it("rejects non-integer strings", () => {
    expect(() => formatUsdt("1.5")).toThrow();
    expect(() => formatUsdt("")).toThrow();
  });
});

describe("parseUsdt", () => {
  it("parses whole and fractional amounts exactly", () => {
    expect(parseUsdt("45")).toBe(45_000_000n);
    expect(parseUsdt(" 0.5 ")).toBe(500_000n);
    expect(parseUsdt("12.345678")).toBe(12_345_678n);
    expect(parseUsdt("1.")).toBe(1_000_000n);
  });

  it("rejects malformed input", () => {
    for (const bad of ["", "-1", "1.2345678", "1,000", "abc", ".5", "1e6"]) {
      expect(parseUsdt(bad)).toBeNull();
    }
  });
});
