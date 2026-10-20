import { describe, expect, it } from "vitest";
import { isExternalNext, safeNext } from "./safe-next";

const WALLET = "https://wallet.example.test";

describe("safeNext", () => {
  it("keeps same-site relative paths", () => {
    expect(safeNext("/t/1?page=2#floor-3", WALLET)).toBe("/t/1?page=2#floor-3");
    expect(safeNext(["/settings", "/x"], WALLET)).toBe("/settings");
  });

  it("returns to the configured wallet origin", () => {
    expect(safeNext(`${WALLET}/order?id=5`, WALLET)).toBe(`${WALLET}/order?id=5`);
    expect(safeNext(`${WALLET}/`, WALLET)).toBe(`${WALLET}/`);
    expect(isExternalNext(safeNext(`${WALLET}/setup`, WALLET))).toBe(true);
  });

  it.each([
    undefined,
    "",
    "//evil.test/x",
    "/\\evil.test",
    "https://evil.test/",
    "https://wallet.example.test.evil.test/",
    "https://wallet.example.test@evil.test/",
    "https://user:pw@wallet.example.test/",
    "http://wallet.example.test/",
    "https://wallet.example.test:8443/",
    "https://WALLET.example.test/",
    "https:\\\\wallet.example.test/",
    "https://wallet.example.test",
    "javascript:alert(1)",
    "data:text/html,x",
    "wallet.example.test/x",
  ])("rejects %s", (raw) => {
    expect(safeNext(raw, WALLET)).toBe("/");
  });

  it("rejects everything absolute when the wallet origin is malformed", () => {
    expect(safeNext(`${WALLET}/x`, "not a url")).toBe("/");
  });
});
