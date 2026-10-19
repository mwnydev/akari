import { describe, expect, it } from "vitest";
import { batchFileName, onTimeRate, percent } from "./arbitration";

describe("arbitration display", () => {
  it("formats bps", () => {
    expect(percent(10_000)).toBe("100%");
    expect(percent(6250)).toBe("62.5%");
    expect(percent(1)).toBe("0.01%");
    expect(percent(0)).toBe("0%");
  });

  it("on-time rate", () => {
    expect(onTimeRate(0, 0)).toBe("—");
    expect(onTimeRate(2, 3)).toBe("67%");
  });

  it("batch file name", () => {
    expect(batchFileName(42161, new Date(2026, 9, 5))).toBe(
      "akari-arbitration-42161-20261005.json",
    );
  });
});
