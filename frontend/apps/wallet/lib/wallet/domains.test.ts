import { describe, expect, it } from "vitest";
import { domainProblems } from "./domains";

const ok = {
  rpId: "wallet.akari.cc",
  apiBase: "https://akari.cc",
  forumOrigin: "https://akari.cc",
};

describe("domainProblems", () => {
  it("accepts the ADR 0012 production domains", () => {
    expect(domainProblems(ok)).toEqual([]);
  });

  it("refuses an RP ID the forum could request", () => {
    for (const rpId of ["akari.cc", "cc", "forum.example"]) {
      const problems = domainProblems({ ...ok, rpId, forumOrigin: "https://forum.example" });
      expect(problems.join()).toMatch(/must not be the forum host/);
    }
    expect(domainProblems({ ...ok, rpId: "localhost" })).toHaveLength(1);
  });

  it("refuses non-https or non-origin URLs", () => {
    for (const apiBase of [
      "",
      "http://akari.cc",
      "https://akari.cc/",
      "https://akari.cc/api",
      "akari.cc",
    ]) {
      expect(domainProblems({ ...ok, apiBase })).toHaveLength(1);
    }
    expect(domainProblems({ ...ok, forumOrigin: "http://localhost:3000" })).toHaveLength(1);
  });
});
