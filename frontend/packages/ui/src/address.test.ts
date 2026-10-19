import { describe, expect, it } from "vitest";
import { shortAddress } from "./address";

describe("shortAddress", () => {
  it("keeps 0x + first 6 and last 6 hex digits", () => {
    expect(shortAddress("0x1234567890abcdef1234567890abcdef12345678")).toBe("0x123456…345678");
  });
  it("rejects non-addresses", () => {
    expect(() => shortAddress("0x1234")).toThrow();
  });
});
