import { describe, expect, it } from "vitest";
import { cnyReference, formatCnyFen, formatCnyPerUsdt, parseCnyFen, parseCnyPerUsdt } from "./fees";

describe("CNY reference rate (ADR 0011)", () => {
  it("parses up to 4 decimals within 5.0000–10.0000", () => {
    expect(parseCnyPerUsdt("7.2")).toBe(72_000n);
    expect(parseCnyPerUsdt(" 7.1234 ")).toBe(71_234n);
    expect(parseCnyPerUsdt("5")).toBe(50_000n);
    expect(parseCnyPerUsdt("10.0000")).toBe(100_000n);
    for (const bad of ["", "72", "0.72", "4.9999", "10.0001", "7.12345", "7,2", "-7"]) {
      expect(parseCnyPerUsdt(bad), bad).toBeNull();
    }
  });

  it("formats the rate and the reference price", () => {
    expect(formatCnyPerUsdt(72_000)).toBe("7.2000");
    expect(cnyReference("45000000", 72_000)).toBe("¥324.00");
    expect(cnyReference("200000000", 71_234)).toBe("¥1,424.68");
    expect(cnyReference("1", 72_000)).toBe("¥0.00");
  });
});

describe("CNY amounts (auctions, ADR 0011)", () => {
  it("parses yuan with up to 2 decimals into fen", () => {
    expect(parseCnyFen("1300")).toBe(130_000n);
    expect(parseCnyFen(" 1300.5 ")).toBe(130_050n);
    expect(parseCnyFen("0.01")).toBe(1n);
    for (const bad of ["", "1,300", "1300.001", "-1", "¥1300", "1e3"]) {
      expect(parseCnyFen(bad), bad).toBeNull();
    }
  });

  it("formats fen with thousands separators", () => {
    expect(formatCnyFen(128_000n)).toBe("¥1,280.00");
    expect(formatCnyFen(12_800_005n)).toBe("¥128,000.05");
    expect(formatCnyFen(0n)).toBe("¥0.00");
  });
});
