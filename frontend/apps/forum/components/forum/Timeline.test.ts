import { describe, expect, it } from "vitest";
import { pointTime } from "./Timeline";

describe("timeline labels", () => {
  it("shows the nearest sampled floor at or before the current one", () => {
    const points = [
      { floor: 1, time: "3 天" },
      { floor: 40, time: "2 天" },
      { floor: 90, time: "5 小时" },
    ];
    expect(pointTime(points, 1)).toBe("3 天");
    expect(pointTime(points, 39)).toBe("3 天");
    expect(pointTime(points, 40)).toBe("2 天");
    expect(pointTime(points, 120)).toBe("5 小时");
    expect(pointTime([], 3)).toBeUndefined();
  });
});
