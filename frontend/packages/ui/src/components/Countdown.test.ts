import { describe, expect, it } from "vitest";
import { formatRemaining } from "./Countdown";

describe("formatRemaining", () => {
  it("formats days, hours and minutes", () => {
    expect(formatRemaining(2 * 86_400 + 3 * 3600 + 59)).toBe("2 天 3 小时");
    expect(formatRemaining(2 * 3600 + 14 * 60 + 5)).toBe("2:14:05");
    expect(formatRemaining(14 * 60 + 5)).toBe("14:05");
  });
  it("clamps negatives and fractions", () => {
    expect(formatRemaining(-5)).toBe("00:00");
    expect(formatRemaining(59.9)).toBe("00:59");
  });
});
