import type { Schemas } from "@akari/api-client";
import { describe, expect, it } from "vitest";
import { caseStatusLabel, caseTimeline, httpUrl } from "./arbitration";

const base = {
  id: 1,
  status: "reviewing",
  panel_size: 1,
  evidence_until: "2026-10-04T00:00:00Z",
  order: { dispute: { by: "buyer", disputed_at: "2026-10-01T00:00:00Z", from_bond: false } },
  assignments: [],
  rulings: [],
  statements: [],
  actions: { seat: "panel", statement: false, appeal: false, halt: false, resume: false },
} as unknown as Schemas["CaseDetail"];

describe("case timeline", () => {
  it("orders past events by time and keeps future deadlines last", () => {
    const c = {
      ...base,
      assignments: [
        {
          arbitrator_id: 5,
          role: "panel",
          assigned_at: "2026-10-04T00:01:00Z",
          due_at: "2026-10-06T00:01:00Z",
          ended_reason: "ruled",
        },
      ],
      rulings: [{ kind: "panel", created_at: "2026-10-05T00:00:00Z" }],
      status: "appeal_window",
      published_at: "2026-10-05T00:00:00Z",
      appeal_until: "2026-10-08T00:00:00Z",
      timeout: { at: "2026-10-31T00:00:00Z", callable: false, to: "0x", data: "0x" },
    } as unknown as Schemas["CaseDetail"];
    const t = caseTimeline(c, new Date("2026-10-06T00:00:00Z"));
    expect(t.map((s) => s.label)).toEqual([
      "发起争议",
      "举证期结束",
      "分配仲裁员",
      "提交仲裁员裁决",
      "裁决公示",
      "申诉期截止",
      "resolveTimeout 兜底可调用",
    ]);
    expect(t.filter((s) => !s.done).map((s) => s.label)).toEqual([
      "申诉期截止",
      "resolveTimeout 兜底可调用",
    ]);
  });

  it("shows appeal, halt and closure", () => {
    const c = {
      ...base,
      status: "halted",
      published_at: "2026-10-05T00:00:00Z",
      appeal_by: 9,
    } as unknown as Schemas["CaseDetail"];
    const labels = caseTimeline(c, new Date("2026-10-06T00:00:00Z")).map((s) => s.label);
    expect(labels).toContain("当事人申诉");
    expect(labels).toContain("站长叫停");
    const closed = { ...c, closed_at: "2026-10-07T00:00:00Z", close_reason: "executed" };
    expect(caseTimeline(closed, new Date()).at(-1)?.label).toBe("已由仲裁 Safe 执行");
    expect(caseStatusLabel(closed)).toBe("已由仲裁 Safe 执行");
  });
});

describe("httpUrl", () => {
  it("keeps only http(s) URLs", () => {
    expect(httpUrl("https://oss.akari.cc/a.webp")).toBe("https://oss.akari.cc/a.webp");
    expect(httpUrl("javascript:alert(1)")).toBeNull();
    expect(httpUrl("data:text/html,x")).toBeNull();
    expect(httpUrl("/relative")).toBeNull();
    expect(httpUrl(null)).toBeNull();
  });
});
