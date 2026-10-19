import { describe, expect, it } from "vitest";
import { revokeWindow } from "./revoke";

describe("revokeWindow", () => {
  it("converts local datetimes to UTC ISO strings", () => {
    const w = revokeWindow("2026-10-01T08:00", "2026-10-02T08:00");
    expect(w).not.toBeNull();
    expect(new Date(w?.to ?? "").getTime() - new Date(w?.from ?? "").getTime()).toBe(86_400_000);
  });

  it("rejects empty, reversed and over-long windows", () => {
    expect(revokeWindow("", "2026-10-02T08:00")).toBeNull();
    expect(revokeWindow("2026-10-02T08:00", "2026-10-02T08:00")).toBeNull();
    expect(revokeWindow("2026-10-02T08:00", "2026-10-01T08:00")).toBeNull();
    expect(revokeWindow("2026-09-01T00:00", "2026-10-02T00:01")).toBeNull();
    expect(revokeWindow("2026-09-01T00:00", "2026-10-02T00:00")).not.toBeNull();
  });
});
